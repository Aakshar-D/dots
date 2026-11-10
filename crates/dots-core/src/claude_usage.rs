//! All of the person's Claude use (usage tab spec): what Claude Code's own transcripts under
//! `<config>/projects` say, read incrementally and read-only, split into Dots' messages and the
//! person's own. No call to any API, and nothing under `<config>` is written.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::model::UsagePeriod;
use crate::scheduler::Clock;
use crate::store::{Store, TranscriptFile};
use crate::util::timestamp;
use crate::{Error, Result};

/// Messages older than this are not kept (the Month period is 30 days).
pub const KEEP_DAYS: i64 = 35;
/// The new bytes one scan reads at most (spec section 5).
pub const SCAN_BYTES: u64 = 32 * 1024 * 1024;
/// The background scan: first this long after start, then every `SCAN_EVERY`.
pub const FIRST_SCAN_AFTER: TimeDelta = TimeDelta::minutes(2);
pub const SCAN_EVERY: TimeDelta = TimeDelta::minutes(10);
/// A scan asked for (the Usage page opening) within this of the last one gets the last one's
/// result.
pub const MIN_GAP: TimeDelta = TimeDelta::seconds(30);
/// The project table's rows at most.
pub const MAX_PROJECTS: u32 = 50;
/// The project row that holds all Dots' messages.
pub const DOTS_ROW: &str = "Dots (all dots)";
/// The 5-hour limit window's name, as the CLI reports it.
const FIVE_HOUR: &str = "five_hour";

/// One assistant message from a transcript line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptMessage {
    pub message_id: String,
    /// Empty when the line had none.
    pub request_id: String,
    /// RFC 3339 UTC with milliseconds, the store's form.
    pub ts: String,
    pub session_id: String,
    pub cwd: String,
    pub model: String,
    pub input_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cache_read_tokens: i64,
    pub output_tokens: i64,
}

/// Messages, sessions and tokens. "Tokens" on the page are input + cache creation + output;
/// cache reads are shown on their own (spec section 6).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeTotals {
    pub messages: u32,
    pub sessions: u32,
    pub input_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cache_read_tokens: i64,
    pub output_tokens: i64,
}

/// One project's messages: `cwd` is the folder the person worked in, `None` for the row that
/// holds all Dots' messages; `name` is the folder's own name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeProject {
    pub cwd: Option<String>,
    pub name: String,
    pub totals: ClaudeTotals,
}

/// The current 5-hour block: the window the CLI last reported, from 5 hours before its reset.
/// Counts every message, Dots' and the person's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeBlock {
    pub started_at: String,
    pub resets_at: String,
    pub messages: u32,
    pub tokens: i64,
}

/// The Usage page's All Claude use section for a period (spec section 7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeUsage {
    pub period: UsagePeriod,
    pub since: String,
    /// The projects folder read, `None` when there is no home folder to look in.
    pub folder: Option<String>,
    /// Whether the folder is there.
    pub available: bool,
    /// When the last scan ended, `None` before the first since start.
    pub scanned_at: Option<String>,
    pub own: ClaudeTotals,
    pub dots: ClaudeTotals,
    pub projects: Vec<ClaudeProject>,
    pub block: Option<ClaudeBlock>,
}

/// What one scan did. `more`: it stopped at the cap with bytes left to read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeScan {
    pub scanned_at: Option<String>,
    pub files: u32,
    pub bytes: u64,
    pub more: bool,
}

#[derive(Deserialize)]
struct Line {
    #[serde(rename = "type")]
    kind: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
    cwd: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    model: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: Option<i64>,
    cache_creation_input_tokens: Option<i64>,
    cache_read_input_tokens: Option<i64>,
    output_tokens: Option<i64>,
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// The assistant message on one transcript line, or `None` for any other line (spec section
/// 3): another type, no usage, no message id or time, the CLI's `<synthetic>` notes, or a line
/// that does not parse.
pub fn parse_line(line: &[u8]) -> Option<TranscriptMessage> {
    // Most bytes are tool results and attachments: skip them before parsing.
    if !contains(line, b"\"assistant\"") || !contains(line, b"\"usage\"") {
        return None;
    }
    let line: Line = serde_json::from_slice(line).ok()?;
    if line.kind.as_deref() != Some("assistant") {
        return None;
    }
    let message = line.message?;
    let usage = message.usage?;
    let model = message.model.unwrap_or_default();
    if model == "<synthetic>" {
        return None;
    }
    let ts = DateTime::parse_from_rfc3339(&line.timestamp?).ok()?;
    Some(TranscriptMessage {
        message_id: message.id.filter(|id| !id.is_empty())?,
        request_id: line.request_id.unwrap_or_default(),
        ts: timestamp(ts.with_timezone(&Utc)),
        session_id: line.session_id.unwrap_or_default(),
        cwd: line.cwd.unwrap_or_default(),
        model,
        input_tokens: usage.input_tokens.unwrap_or(0),
        cache_creation_tokens: usage.cache_creation_input_tokens.unwrap_or(0),
        cache_read_tokens: usage.cache_read_input_tokens.unwrap_or(0),
        output_tokens: usage.output_tokens.unwrap_or(0),
    })
}

/// What reading one file from an offset found.
#[derive(Debug, Default)]
pub struct FileRead {
    pub messages: Vec<TranscriptMessage>,
    /// The bytes of complete lines read; the next read starts after them.
    pub read: u64,
    /// The read stopped at `budget` rather than at the end of the file.
    pub capped: bool,
}

/// Reads `path` from `start`, complete lines only, until the end or until at least `budget`
/// bytes were read (a line is never cut). Messages sent before `since` are left out.
pub fn read_from(path: &Path, start: u64, budget: u64, since: &str) -> std::io::Result<FileRead> {
    let mut file = std::fs::File::open(path)?;
    file.seek(SeekFrom::Start(start))?;
    let mut reader = BufReader::with_capacity(64 * 1024, file);
    let mut out = FileRead::default();
    let mut buf = Vec::new();
    loop {
        if out.read >= budget {
            out.capped = true;
            break;
        }
        buf.clear();
        let n = reader.read_until(b'\n', &mut buf)?;
        // The end, or a last line still being written: read next time.
        if n == 0 || buf.last() != Some(&b'\n') {
            break;
        }
        out.read += n as u64;
        if let Some(m) = parse_line(&buf) {
            if m.ts.as_str() >= since {
                out.messages.push(m);
            }
        }
    }
    Ok(out)
}

/// One transcript file found: its path, size and modification time (Unix ms).
#[derive(Debug, Clone)]
struct Found {
    path: PathBuf,
    size: u64,
    mtime_ms: i64,
}

fn found(path: PathBuf) -> Option<Found> {
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as i64);
    Some(Found {
        path,
        size: meta.len(),
        mtime_ms,
    })
}

fn jsonl_in(dir: &Path, out: &mut Vec<Found>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "jsonl") {
            out.extend(found(path));
        }
    }
}

/// Every transcript under `projects`: `<project>/*.jsonl` and
/// `<project>/<session>/subagents/*.jsonl`, newest first.
fn list_files(projects: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    let Ok(projects) = std::fs::read_dir(projects) else {
        return out;
    };
    for project in projects.flatten() {
        let dir = project.path();
        if !dir.is_dir() {
            continue;
        }
        jsonl_in(&dir, &mut out);
        let Ok(sessions) = std::fs::read_dir(&dir) else {
            continue;
        };
        for session in sessions.flatten() {
            let sub = session.path().join("subagents");
            if sub.is_dir() {
                jsonl_in(&sub, &mut out);
            }
        }
    }
    out.sort_by(|a, b| b.mtime_ms.cmp(&a.mtime_ms).then(a.path.cmp(&b.path)));
    out
}

/// A path as compared for "inside": forward slashes, no verbatim prefix, no trailing slash,
/// and lower case on Windows.
fn norm(p: &str) -> String {
    let p = p.strip_prefix(r"\\?\").unwrap_or(p).replace('\\', "/");
    let p = p.trim_end_matches('/').to_string();
    if cfg!(windows) {
        p.to_lowercase()
    } else {
        p
    }
}

/// Whether `cwd` is `dir` or inside it.
pub fn is_inside(cwd: &str, dir: &str) -> bool {
    let (cwd, dir) = (norm(cwd), norm(dir));
    !dir.is_empty() && (cwd == dir || cwd.starts_with(&format!("{dir}/")))
}

/// The last part of a path, either separator; the whole path when it has none.
pub fn folder_name(path: &str) -> String {
    path.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// Where Claude Code keeps its transcripts: `CLAUDE_CONFIG_DIR/projects` when that is set, else
/// `<home>/.claude/projects`.
pub fn projects_dir(config_dir: Option<&str>, home: Option<&Path>) -> Option<PathBuf> {
    match config_dir.map(str::trim).filter(|d| !d.is_empty()) {
        Some(dir) => Some(PathBuf::from(dir).join("projects")),
        None => home.map(|h| h.join(".claude").join("projects")),
    }
}

/// Reads the transcripts and answers the Usage page's All Claude use section.
pub struct ClaudeUsageService {
    store: Store,
    projects: Option<PathBuf>,
    /// Dots' data folder: a message whose cwd is inside it is Dots'.
    dots_dir: String,
    clock: Arc<dyn Clock>,
    /// The last scan's end and result; held while a scan runs, so scans go one at a time.
    last: tokio::sync::Mutex<Option<(DateTime<Utc>, ClaudeScan)>>,
    /// When the last scan ended, readable while the next one runs.
    scanned_at: std::sync::Mutex<Option<String>>,
}

impl ClaudeUsageService {
    pub fn new(
        store: Store,
        projects: Option<PathBuf>,
        dots_dir: &Path,
        clock: Arc<dyn Clock>,
    ) -> Arc<Self> {
        let dots_dir = std::path::absolute(dots_dir).unwrap_or_else(|_| dots_dir.to_path_buf());
        Arc::new(Self {
            store,
            projects,
            dots_dir: dots_dir.to_string_lossy().to_string(),
            clock,
            last: tokio::sync::Mutex::new(None),
            scanned_at: std::sync::Mutex::new(None),
        })
    }

    /// Scans unless the last scan ended within `min_gap` and read everything; then that scan's
    /// result is the answer. A scan asked for while one runs waits for it.
    pub async fn scan(&self, min_gap: TimeDelta) -> Result<ClaudeScan> {
        let mut last = self.last.lock().await;
        if let Some((at, result)) = last.as_ref() {
            if !result.more && self.clock.now() - *at < min_gap {
                return Ok(result.clone());
            }
        }
        let result = self.scan_once().await?;
        *last = Some((self.clock.now(), result.clone()));
        if let Ok(mut at) = self.scanned_at.lock() {
            at.clone_from(&result.scanned_at);
        }
        Ok(result)
    }

    async fn scan_once(&self) -> Result<ClaudeScan> {
        let Some(projects) = self.projects.clone() else {
            return Ok(ClaudeScan::default());
        };
        let now = self.clock.now();
        let since = now - TimeDelta::days(KEEP_DAYS);
        let since_ts = timestamp(since);
        let files = tokio::task::spawn_blocking(move || list_files(&projects))
            .await
            .map_err(|e| Error::Other(format!("listing Claude Code's transcripts failed: {e}")))?;
        let known = self.store.transcript_files().await?;
        let sessions = self.store.run_session_ids().await?;
        let scanned_at = timestamp(now);
        let mut result = ClaudeScan::default();
        let mut budget = SCAN_BYTES;
        let mut seen: HashSet<String> = HashSet::new();
        for f in files {
            let key = f.path.to_string_lossy().to_string();
            seen.insert(key.clone());
            // A file smaller than what was read was rewritten: read it again from the start.
            let start = match known.get(&key) {
                Some(k) if k.offset <= f.size => k.offset,
                _ => 0,
            };
            if start == f.size && known.contains_key(&key) {
                continue;
            }
            let mut record = TranscriptFile {
                path: key,
                size: f.size,
                mtime_ms: f.mtime_ms,
                offset: f.size,
            };
            // Last written before anything kept: mark it read without opening it.
            if f.mtime_ms < since.timestamp_millis() {
                self.store
                    .save_transcript_read(&record, &[], &scanned_at)
                    .await?;
                continue;
            }
            if budget == 0 {
                result.more = true;
                break;
            }
            let (path, cap, from) = (f.path.clone(), budget, since_ts.clone());
            let read = tokio::task::spawn_blocking(move || read_from(&path, start, cap, &from))
                .await
                .map_err(|e| Error::Other(format!("reading a transcript failed: {e}")))?;
            let read = match read {
                Ok(read) => read,
                Err(e) => {
                    tracing::debug!(path = %f.path.display(), "a transcript could not be read: {e}");
                    continue;
                }
            };
            record.offset = start + read.read;
            let messages: Vec<(TranscriptMessage, bool)> = read
                .messages
                .into_iter()
                .map(|m| {
                    let dots =
                        sessions.contains(&m.session_id) || is_inside(&m.cwd, &self.dots_dir);
                    (m, dots)
                })
                .collect();
            self.store
                .save_transcript_read(&record, &messages, &scanned_at)
                .await?;
            budget = budget.saturating_sub(read.read);
            result.bytes += read.read;
            result.files += 1;
            if read.capped {
                result.more = true;
            }
        }
        let gone: Vec<String> = known.into_keys().filter(|k| !seen.contains(k)).collect();
        if !gone.is_empty() && !result.more {
            self.store.forget_transcript_files(&gone).await?;
        }
        self.store.prune_claude_messages(&since_ts).await?;
        result.scanned_at = Some(timestamp(self.clock.now()));
        Ok(result)
    }

    /// The All Claude use section for `period`.
    pub async fn summary(&self, period: UsagePeriod) -> Result<ClaudeUsage> {
        let now = self.clock.now();
        let since = timestamp(now - period.reach());
        let (own, dots) = self.store.claude_totals(&since).await?;
        let projects = self.store.claude_projects(&since, MAX_PROJECTS).await?;
        let scanned_at = self.scanned_at.lock().ok().and_then(|at| at.clone());
        let available = match &self.projects {
            Some(p) => {
                let p = p.clone();
                tokio::task::spawn_blocking(move || p.is_dir())
                    .await
                    .unwrap_or(false)
            }
            None => false,
        };
        Ok(ClaudeUsage {
            period,
            since,
            folder: self
                .projects
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            available,
            scanned_at,
            own,
            dots,
            projects,
            block: self.block(now).await?,
        })
    }

    /// The current 5-hour block, when the CLI's last 5-hour reading has not reset yet.
    async fn block(&self, now: DateTime<Utc>) -> Result<Option<ClaudeBlock>> {
        let readings = self.store.limit_readings().await?;
        let Some(resets) = readings
            .iter()
            .find(|r| r.window == FIVE_HOUR)
            .and_then(|r| DateTime::parse_from_rfc3339(&r.resets_at).ok())
            .map(|t| t.with_timezone(&Utc))
            .filter(|t| *t > now)
        else {
            return Ok(None);
        };
        let started = resets - TimeDelta::hours(5);
        let started_at = timestamp(started);
        let (messages, tokens) = self.store.claude_messages_since(&started_at).await?;
        Ok(Some(ClaudeBlock {
            started_at,
            resets_at: timestamp(resets),
            messages,
            tokens,
        }))
    }

    /// Scans in the background: first `FIRST_SCAN_AFTER` after start, then every `SCAN_EVERY`,
    /// until shutdown. Nothing to do without a projects folder.
    pub fn spawn(self: &Arc<Self>, shutdown: CancellationToken) {
        if self.projects.is_none() {
            return;
        }
        let this = Arc::downgrade(self);
        let clock = self.clock.clone();
        tokio::spawn(async move {
            let mut next = clock.now() + FIRST_SCAN_AFTER;
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = clock.sleep_until(next) => {}
                }
                next = clock.now() + SCAN_EVERY;
                let Some(service) = this.upgrade() else {
                    return;
                };
                if let Err(e) = service.scan(TimeDelta::zero()).await {
                    tracing::warn!("reading Claude Code's transcripts failed: {e}");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, req: &str, out: i64) -> String {
        format!(
            r#"{{"type":"assistant","timestamp":"2026-11-10T10:00:00.000Z","sessionId":"s1","cwd":"C:\\work\\app","requestId":"{req}","message":{{"id":"{id}","model":"claude-opus-5-5","role":"assistant","content":[{{"type":"text","text":"hi"}}],"usage":{{"input_tokens":3,"cache_creation_input_tokens":100,"cache_read_input_tokens":2000,"output_tokens":{out},"service_tier":"standard"}}}}}}"#
        )
    }

    #[test]
    fn an_assistant_line_gives_its_message() {
        let m = parse_line(line("msg_1", "req_1", 40).as_bytes()).unwrap();
        assert_eq!(m.message_id, "msg_1");
        assert_eq!(m.request_id, "req_1");
        assert_eq!(m.ts, "2026-11-10T10:00:00.000Z");
        assert_eq!(m.session_id, "s1");
        assert_eq!(m.cwd, r"C:\work\app");
        assert_eq!(m.model, "claude-opus-5-5");
        assert_eq!(
            (
                m.input_tokens,
                m.cache_creation_tokens,
                m.cache_read_tokens,
                m.output_tokens
            ),
            (3, 100, 2000, 40)
        );
    }

    #[test]
    fn other_lines_give_nothing() {
        let user = r#"{"type":"user","timestamp":"2026-11-10T10:00:00.000Z","message":{"role":"user","content":"assistant usage"}}"#;
        assert_eq!(parse_line(user.as_bytes()), None);
        let synthetic = line("msg_1", "r", 0).replace("claude-opus-5-5", "<synthetic>");
        assert_eq!(parse_line(synthetic.as_bytes()), None);
        let no_usage = r#"{"type":"assistant","timestamp":"2026-11-10T10:00:00.000Z","message":{"id":"m","model":"x"}}"#;
        assert_eq!(parse_line(no_usage.as_bytes()), None);
        let no_id = line("", "r", 1);
        assert_eq!(parse_line(no_id.as_bytes()), None);
        assert_eq!(
            parse_line(b"{\"type\":\"assistant\",\"usage\" not json"),
            None
        );
        let bad_time = line("m", "r", 1).replace("2026-11-10T10:00:00.000Z", "yesterday");
        assert_eq!(parse_line(bad_time.as_bytes()), None);
    }

    #[test]
    fn a_line_without_a_request_id_keeps_an_empty_one() {
        let l = line("m", "r", 1).replace(r#""requestId":"r","#, "");
        assert_eq!(parse_line(l.as_bytes()).unwrap().request_id, "");
    }

    #[test]
    fn inside_compares_whole_folders_in_either_separator() {
        assert!(is_inside(
            r"C:\Users\me\AppData\Dots\worktrees\x",
            "C:/Users/me/AppData/Dots"
        ));
        assert!(is_inside(
            "C:/Users/me/AppData/Dots",
            r"C:\Users\me\AppData\Dots\"
        ));
        assert!(!is_inside(
            r"C:\Users\me\AppData\Dots2",
            r"C:\Users\me\AppData\Dots"
        ));
        assert!(!is_inside(r"C:\anything", ""));
        #[cfg(windows)]
        assert!(is_inside(
            r"c:\users\ME\appdata\dots\x",
            r"C:\Users\me\AppData\Dots"
        ));
    }

    #[test]
    fn a_folder_name_is_the_last_part() {
        assert_eq!(folder_name(r"C:\Users\me\code\dots"), "dots");
        assert_eq!(folder_name("/home/me/code/app/"), "app");
        assert_eq!(folder_name("C:\\"), "C:");
    }

    #[test]
    fn the_projects_folder_follows_claude_config_dir() {
        let home = Path::new("/home/me");
        assert_eq!(
            projects_dir(Some("/cfg"), Some(home)),
            Some(PathBuf::from("/cfg").join("projects"))
        );
        assert_eq!(
            projects_dir(Some("  "), Some(home)),
            Some(home.join(".claude").join("projects"))
        );
        assert_eq!(projects_dir(None, None), None);
    }
}
