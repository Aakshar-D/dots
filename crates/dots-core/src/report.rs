//! The problem report (report a problem spec): a plain-text page the person copies or saves and
//! sends themselves. It holds their own words, a few facts about this copy of Dots, and the
//! recent log lines, scrubbed again. Nothing a dot said, was told or worked on is read for it.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::{DateTime, FixedOffset, Local, Utc};
use regex::Regex;

use crate::store::Store;
use crate::vault::redact;
use crate::Result;

/// How many log lines a report carries at most.
pub const LOG_LINES: usize = 200;
/// How much of the end of each log file is read: a day's log can be large, and a report only
/// wants its last lines.
const TAIL_BYTES: u64 = 2 * 1024 * 1024;

/// The facts about this copy of Dots that a report carries: no names, no paths, no text.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportFacts {
    pub app_version: String,
    pub os: String,
    pub arch: String,
    /// The engine kinds the dots use (`claude`, `local`), each once.
    pub engines: Vec<String>,
    pub dots: u32,
    /// Runs queued in the last 24 hours, and how many of them failed.
    pub runs_24h: u32,
    pub failed_24h: u32,
}

/// The log part of a report.
#[derive(Debug, Clone, PartialEq)]
pub enum LogSection {
    /// The person left "Include recent log lines" off.
    NotIncluded,
    /// No log folder, or no log file in it.
    NoLog,
    /// Scrubbed lines, oldest first, and how many lines of servers' and the browser helper's
    /// output were left out among them.
    Lines { lines: Vec<String>, left_out: usize },
}

/// A log line's start: `<time>  <LEVEL> <target>: <message> <fields>`. Group 1 is the head up
/// to the target's colon, 2 the target, 3 the message.
static LINE_HEAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\d{4}-\d{2}-\d{2}T\S+\s+[A-Z]+\s+(\S+?):)\s(.*)$")
        .expect("the line head compiles")
});

/// Whether `line` is output of a program Dots does not control, which may echo what it was
/// given: an MCP server's stderr, or the browser helper's stderr (spec §3).
pub fn is_foreign_output(line: &str) -> bool {
    let Some(head) = LINE_HEAD.captures(line) else {
        return false;
    };
    let (target, message) = (&head[2], &head[3]);
    match target {
        "dots_core::mcp::stdio" => message.starts_with("MCP server stderr:"),
        // The helper's lines are logged bare with the dot's id; Dots' own lines there say
        // what it could not do, and carry no `dot` field.
        "dots_core::browser::host" => message.contains(" dot=") || message.starts_with("dot="),
        _ => false,
    }
}

/// What a line whose message is hidden says in its place.
pub const MESSAGE_HIDDEN: &str = "[message hidden]";

/// Whether the message of a line from `target` may carry text of an engine, a tool, a plugin
/// or a run (spec §3): the engines' notices and errors, a failed run's error (it holds the
/// engine's stderr tail), a follow-up's start error, plugin and hook warnings, an HTTP MCP
/// server's answers, and the browser host's download names.
fn message_is_hidden(target: &str, message: &str) -> bool {
    target == "dots_core::engine"
        || target.starts_with("dots_core::engine::")
        || target == "dots_core::library_service"
        || target == "dots_core::mcp::http"
        || target == "dots_core::browser::host"
        || (target == "dots_core::runner" && message.starts_with("run failed:"))
        || (target == "dots_core::followups"
            && message.starts_with("the follow-up's run could not be started"))
}

/// The patterns of secrets in free text, each with what takes its place, in order. A key
/// prefix counts only at the start of a word, so `task-…` is not `sk-…`.
static SECRETS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"-----BEGIN[^\r\n]*", "[secret]"),
        // Credentials in a URL, before the email rule sees `pw@host`.
        (r"://[^/\s@:]+:[^/\s@]*@", "://[secret]@"),
        (r"\beyJ[\w-]+\.[\w-]+\.[\w-]*", "[secret]"),
        (
            r"(?i)([?&](?:sig|signature|code|key|auth|token|access_token|client_secret|password|pw)=)[^&\s#]+",
            "${1}[secret]",
        ),
        (
            r"(^|[^A-Za-z0-9_-])(?:sk-ant-|sk-|sk_live_|ghp_|gho_|ghs_|github_pat_|xox[abpr]-|AKIA|ASIA)[A-Za-z0-9_-]{16,}",
            "${1}[secret]",
        ),
        (r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]{8,}", "Bearer [secret]"),
        (
            r#"(?i)(authorization["']?\s*[:=]\s*["']?)[^"'\r\n,}]+"#,
            "${1}[secret]",
        ),
        // Not inside a `[secret:NAME]` the redactor wrote.
        (
            r#"(?i)(^|[^\[A-Za-z_-])([A-Za-z_-]*(?:password|passwd|secret|token|api[_-]?key)["']?\s*[:=]\s*["']?)[^\s"',;&}]+"#,
            "${1}${2}[secret]",
        ),
        // A long hex or base64 run after a word that names a key, a secret or a token.
        (
            r"(?i)(\b(?:key|secret|token)s?\b[^A-Za-z0-9\r\n]{1,3})[A-Za-z0-9+/_-]{32,}={0,2}",
            "${1}[secret]",
        ),
    ]
    .into_iter()
    .map(|(pattern, with)| (Regex::new(pattern).expect("the secret patterns compile"), with))
    .collect()
});

/// An email address, with or without a dot in its domain (`asha@localhost`).
static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*")
        .expect("the email pattern compiles")
});

/// A path: one with a drive letter (any slashes, the doubled backslashes of debug output, a
/// short `ASHAKU~1` part), a UNC path, or an absolute path under a Unix home. A space goes on
/// into the path when the next word holds a slash, so `My Projects\x` is one path. Group 1 is
/// what comes before it, group 2 the path.
static PATH: LazyLock<Regex> = LazyLock::new(|| {
    let part = r#"[^\s"'<>|,;:*?]"#;
    let more = format!(r"(?: [^\s\\/]+[\\/]{part}*)*");
    Regex::new(&format!(
        r"(^|[^A-Za-z0-9])((?:[A-Za-z]:[\\/]{part}*{more})|(?:\\\\{part}+{more})|(?:/(?:home|Users|root)/{part}*{more}))"
    ))
    .expect("the path pattern compiles")
});

/// The extension of the last part of `path`, if it has one (`.rs`).
fn extension(path: &str) -> &str {
    let last = path.rsplit(['\\', '/']).next().unwrap_or("");
    match last.rfind('.') {
        Some(i)
            if i > 0
                && i + 1 < last.len()
                && last.len() - i <= 9
                && last[i + 1..].chars().all(|c| c.is_ascii_alphanumeric()) =>
        {
            &last[i..]
        }
        _ => "",
    }
}

/// `text` with the secrets the vault has read and anything shaped like a secret replaced.
pub fn scrub_text(text: &str) -> String {
    let mut out = redact::global().scrub(text).into_owned();
    for (pattern, with) in SECRETS.iter() {
        if pattern.is_match(&out) {
            out = pattern.replace_all(&out, *with).into_owned();
        }
    }
    out
}

/// The spellings of the home folder a log may hold: as it is, with forward slashes, and with
/// doubled backslashes (a path in debug output), in any letter case.
fn home_pattern(home: &str) -> Option<Regex> {
    let home = home.trim_end_matches(['\\', '/']);
    if home.len() < 4 {
        return None;
    }
    let parts: Vec<String> = home.split(['\\', '/']).map(regex::escape).collect();
    Regex::new(&format!(r"(?i){}", parts.join(r"(?:\\\\|\\|/)"))).ok()
}

/// A field that names a file the person attached or a watcher held: `file=<name>`. The name
/// runs to the next field (` key=`) or the end of the line, spaces and all.
static FILE_FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\bfile=").expect("the file field compiles"));
static NEXT_FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r" [a-z_]+=").expect("the next field compiles"));

/// `text` with the value of every `file=` field as `[file]`.
fn hide_file_fields(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(m) = FILE_FIELD.find(rest) {
        out.push_str(&rest[..m.end()]);
        out.push_str("[file]");
        let after = &rest[m.end()..];
        rest = match NEXT_FIELD.find(after) {
            Some(next) => &after[next.start()..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Scrubs log lines for a report: secrets, the names of the dots, workflows and teams, file
/// names, paths, emails and the home folder (spec §3).
pub struct Scrubber {
    /// Every name to hide, in lower case, with what takes its place.
    labels: HashMap<String, &'static str>,
    names: Option<Regex>,
    home: Option<Regex>,
}

impl Scrubber {
    /// `dot_names` are the names of every dot, hidden as `[dot]`; `home` the home folder.
    pub fn new(dot_names: &[String], home: Option<&str>) -> Scrubber {
        let mut s = Scrubber {
            labels: HashMap::new(),
            names: None,
            home: home.and_then(home_pattern),
        };
        s.add_names(dot_names, "[dot]");
        s
    }

    /// Hides `names` as `label` too (`[workflow]`, `[team]`).
    pub fn hiding(mut self, names: &[String], label: &'static str) -> Scrubber {
        self.add_names(names, label);
        self
    }

    fn add_names(&mut self, names: &[String], label: &'static str) {
        for name in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
            self.labels.entry(name.to_lowercase()).or_insert(label);
        }
        let mut all: Vec<&str> = self.labels.keys().map(String::as_str).collect();
        // Longest first, so a name that holds another is hidden whole.
        all.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        let alternatives: Vec<String> = all
            .iter()
            .map(|n| {
                let start = if word(n.chars().next()) { r"\b" } else { "" };
                let end = if word(n.chars().last()) { r"\b" } else { "" };
                format!("{start}{}{end}", regex::escape(n))
            })
            .collect();
        self.names = if alternatives.is_empty() {
            None
        } else {
            Regex::new(&format!("(?i)(?:{})", alternatives.join("|"))).ok()
        };
    }

    /// A log line as a report carries it: `scrub_text`, file fields as `[file]`, the names as
    /// their labels, paths as `[path]` with their extension, emails as `[email]`, and what is
    /// left of the home folder as `~`.
    pub fn line(&self, line: &str) -> String {
        let mut out = hide_file_fields(&scrub_text(line));
        if let Some(names) = &self.names {
            out = names
                .replace_all(&out, |c: &regex::Captures| {
                    self.labels
                        .get(&c[0].to_lowercase())
                        .copied()
                        .unwrap_or("[dot]")
                })
                .into_owned();
        }
        out = PATH
            .replace_all(&out, |c: &regex::Captures| {
                format!("{}[path]{}", &c[1], extension(&c[2]))
            })
            .into_owned();
        out = EMAIL.replace_all(&out, "[email]").into_owned();
        if let Some(home) = &self.home {
            out = home.replace_all(&out, "~").into_owned();
        }
        out
    }
}

/// `line` scrubbed with no dot names and `home` as the home folder.
pub fn scrub_line(line: &str, home: Option<&str>) -> String {
    Scrubber::new(&[], home).line(line)
}

/// The log files in `dir`, newest first (`dots.YYYY-MM-DD.log` sorts by date).
fn log_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("sols.") && n.ends_with(".log"))
        })
        .collect();
    files.sort();
    files.reverse();
    files
}

/// The last `TAIL_BYTES` of `path` as lines; a line cut by the start of the window is dropped.
fn tail_of(path: &Path) -> Vec<String> {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let len = file.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }
    lines
}

/// One line of a log as a report treats it.
enum Entry {
    /// A line to scrub and keep.
    Keep(String),
    /// A line already made safe (a hidden message, a count of hidden lines): kept as it is.
    Safe(String),
    /// Output of another program, left out and counted.
    LeftOut,
}

/// What the lines that follow, without a log line's start of their own, belong to.
enum Mode {
    Normal,
    Foreign,
    /// Inside a private key block, until its `-----END` line.
    Key,
    /// The rest of a panic or of a hidden message: counted, then one note.
    Collapse(usize),
}

/// Keeps `line`, or, when a private key block starts in it, what comes before the block.
fn keep(line: String, out: &mut Vec<Entry>) -> Mode {
    match line.find("-----BEGIN") {
        Some(at) if !line[at..].contains("-----END") => {
            out.push(Entry::Safe(format!("{}[private key hidden]", &line[..at])));
            Mode::Key
        }
        _ => {
            out.push(Entry::Keep(line));
            Mode::Normal
        }
    }
}

fn flush(mode: &Mode, out: &mut Vec<Entry>) {
    if let Mode::Collapse(n) = mode {
        if *n > 0 {
            out.push(Entry::Safe(format!("[{n} more lines hidden]")));
        }
    }
}

/// One file's lines, sorted (spec §3). A line without a log line's start continues the line
/// before it and goes the same way; a private key block is dropped through its end line.
fn sort_lines(lines: Vec<String>) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut mode = Mode::Normal;
    for line in lines {
        if let Mode::Key = mode {
            if line.contains("-----END") {
                mode = Mode::Normal;
            }
            continue;
        }
        let head = LINE_HEAD
            .captures(&line)
            .map(|h| (h[1].to_string(), h[2].to_string(), h[3].to_string()));
        if let Some((start, target, message)) = head {
            flush(&mode, &mut out);
            mode = if is_foreign_output(&line) {
                out.push(Entry::LeftOut);
                Mode::Foreign
            } else if message_is_hidden(&target, &message) {
                out.push(Entry::Safe(format!("{start} {MESSAGE_HIDDEN}")));
                Mode::Collapse(0)
            } else if message.contains("panicked at ") {
                match keep(line, &mut out) {
                    Mode::Key => Mode::Key,
                    _ => Mode::Collapse(0),
                }
            } else {
                keep(line, &mut out)
            };
            continue;
        }
        mode = match mode {
            Mode::Foreign => {
                out.push(Entry::LeftOut);
                Mode::Foreign
            }
            Mode::Collapse(n) => Mode::Collapse(n + 1),
            Mode::Normal | Mode::Key => keep(line, &mut out),
        };
    }
    flush(&mode, &mut out);
    out
}

/// The last `max` lines of the logs in `dir`, scrubbed with `scrubber`, read back across files
/// when the newest is short.
pub fn read_log_tail(dir: &Path, max: usize, scrubber: &Scrubber) -> LogSection {
    let files = log_files(dir);
    if files.is_empty() {
        return LogSection::NoLog;
    }
    // Newest last, oldest first: older files are put in front as they are read.
    let mut sorted: Vec<Entry> = Vec::new();
    for file in files {
        let mut older = sort_lines(tail_of(&file));
        older.append(&mut sorted);
        sorted = older;
        if sorted
            .iter()
            .filter(|e| !matches!(e, Entry::LeftOut))
            .count()
            >= max
        {
            break;
        }
    }
    let mut lines = Vec::new();
    let mut left_out = 0;
    for entry in sorted.into_iter().rev() {
        if lines.len() == max {
            break;
        }
        match entry {
            Entry::Keep(line) => lines.push(scrubber.line(&line)),
            Entry::Safe(line) => lines.push(scrubber.line(&line)),
            Entry::LeftOut => left_out += 1,
        }
    }
    lines.reverse();
    LogSection::Lines { lines, left_out }
}

/// The report's text (spec §4). `description` is the person's own text: trimmed and scrubbed
/// of secrets.
pub fn render(
    facts: &ReportFacts,
    description: &str,
    logs: &LogSection,
    made: DateTime<FixedOffset>,
) -> String {
    let description = description.trim();
    let description = if description.is_empty() {
        "(not described)".to_string()
    } else {
        scrub_text(description)
    };
    let engines = if facts.engines.is_empty() {
        "none".to_string()
    } else {
        facts.engines.join(", ")
    };
    let mut out = format!(
        "Sage problem report\n\
         Made: {made}\n\
         \n\
         == What went wrong ==\n\
         {description}\n\
         \n\
         == This copy of Sage ==\n\
         Version: {version}\n\
         System: {os} {arch}\n\
         Engines in use: {engines}\n\
         Sage: {dots}\n\
         Runs in the last 24 hours: {runs} ({failed} failed)\n\
         \n",
        made = made.format("%Y-%m-%d %H:%M (%:z)"),
        version = facts.app_version,
        os = facts.os,
        arch = facts.arch,
        dots = facts.dots,
        runs = facts.runs_24h,
        failed = facts.failed_24h,
    );
    match logs {
        LogSection::NotIncluded => out.push_str("== Recent log lines ==\n(not included)\n"),
        LogSection::NoLog => out.push_str("== Recent log lines ==\n(no log found)\n"),
        LogSection::Lines { lines, left_out } => {
            out.push_str(&format!(
                "== Recent log lines ({}, scrubbed) ==\n",
                lines.len()
            ));
            for line in lines {
                out.push_str(line);
                out.push('\n');
            }
            if *left_out > 0 {
                out.push_str(&format!(
                    "({left_out} lines from MCP servers and the browser helper left out)\n"
                ));
            }
        }
    }
    out
}

/// The name the save dialog offers: `dots-report-YYYYMMDD-HHMM.txt`, in local time.
pub fn file_name(made: DateTime<FixedOffset>) -> String {
    format!("dots-report-{}.txt", made.format("%Y%m%d-%H%M"))
}

/// The person's home folder, for `scrub_line`.
pub fn home_dir() -> Option<String> {
    std::env::var("USERPROFILE")
        .ok()
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("HOME").ok().filter(|h| !h.is_empty()))
}

/// The facts about this copy of Dots: `app_version`, the system, and the store's counts over
/// the last 24 hours.
pub async fn facts(store: &Store, app_version: &str) -> Result<ReportFacts> {
    let counts = store
        .report_counts(Utc::now() - chrono::Duration::hours(24))
        .await?;
    Ok(ReportFacts {
        app_version: app_version.to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        engines: counts.engines,
        dots: counts.dots,
        runs_24h: counts.runs,
        failed_24h: counts.failed,
    })
}

/// The whole report, made now in local time: the facts, the person's `description`, and, when
/// `include_logs`, the last `LOG_LINES` lines of the logs in `log_dir` (none when there is no
/// log folder).
pub async fn build(
    store: &Store,
    app_version: &str,
    log_dir: Option<&Path>,
    description: &str,
    include_logs: bool,
) -> Result<String> {
    let facts = facts(store, app_version).await?;
    let logs = match (include_logs, log_dir) {
        (false, _) => LogSection::NotIncluded,
        (true, None) => LogSection::NoLog,
        (true, Some(dir)) => {
            let dir = dir.to_path_buf();
            let scrubber = Scrubber::new(&store.dot_names().await?, home_dir().as_deref())
                .hiding(&store.workflow_names().await?, "[workflow]")
                .hiding(&store.team_names().await?, "[team]");
            tokio::task::spawn_blocking(move || read_log_tail(&dir, LOG_LINES, &scrubber))
                .await
                .unwrap_or(LogSection::NoLog)
        }
    };
    Ok(render(
        &facts,
        description,
        &logs,
        Local::now().fixed_offset(),
    ))
}
