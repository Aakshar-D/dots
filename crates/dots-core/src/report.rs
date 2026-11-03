//! The problem report (report a problem spec): a plain-text page the person copies or saves and
//! sends themselves. It holds their own words, a few facts about this copy of Dots, and the
//! recent log lines, scrubbed again. Nothing a dot said, was told or worked on is read for it.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::{DateTime, FixedOffset};
use regex::Regex;

use crate::vault::redact;

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

/// A log line's start: `<time>  <LEVEL> <target>: <message> <fields>`.
static LINE_HEAD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^\d{4}-\d{2}-\d{2}T\S+\s+[A-Z]+\s+(\S+?):\s(.*)$").expect("the line head compiles")
});

/// Whether `line` is output of a program Dots does not control, which may echo what it was
/// given: an MCP server's stderr, or the browser helper's stderr (spec §3).
pub fn is_foreign_output(line: &str) -> bool {
    let Some(head) = LINE_HEAD.captures(line) else {
        return false;
    };
    let (target, message) = (&head[1], &head[2]);
    match target {
        "dots_core::mcp::stdio" => message.starts_with("MCP server stderr:"),
        // The helper's lines are logged bare with the dot's id; Dots' own lines there say
        // what it could not do, and carry no `dot` field.
        "dots_core::browser::host" => message.contains(" dot=") || message.starts_with("dot="),
        _ => false,
    }
}

/// The patterns of secrets in free text, each with what takes its place. A key prefix counts
/// only at the start of a word, so `task-…` is not `sk-…`.
static SECRETS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"-----BEGIN[^\r\n]*", "[secret]"),
        (
            r"(^|[^A-Za-z0-9_-])(?:sk-|sk_live_|ghp_|gho_|github_pat_|xox[bpas]-|AKIA)[A-Za-z0-9_-]{16,}",
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
    ]
    .into_iter()
    .map(|(pattern, with)| (Regex::new(pattern).expect("the secret patterns compile"), with))
    .collect()
});

static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}")
        .expect("the email pattern compiles")
});

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

/// A log line as a report carries it: `scrub_text`, then emails as `[email]` and the home
/// folder `home` as `~`.
pub fn scrub_line(line: &str, home: Option<&str>) -> String {
    let out = scrub_text(line);
    let out = EMAIL.replace_all(&out, "[email]").into_owned();
    match home.and_then(home_pattern) {
        Some(pattern) => pattern.replace_all(&out, "~").into_owned(),
        None => out,
    }
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
                    .is_some_and(|n| n.starts_with("dots.") && n.ends_with(".log"))
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

/// One file's lines: each kept line (raw), or `None` for a line left out. A line without a
/// log line's start continues the line before it and goes the same way.
fn sort_lines(lines: Vec<String>) -> Vec<Option<String>> {
    let mut dropping = false;
    lines
        .into_iter()
        .map(|line| {
            if LINE_HEAD.is_match(&line) {
                dropping = is_foreign_output(&line);
            }
            (!dropping).then_some(line)
        })
        .collect()
}

/// The last `max` lines of the logs in `dir`, scrubbed with `home` as the home folder, read
/// back across files when the newest is short.
pub fn read_log_tail(dir: &Path, max: usize, home: Option<&str>) -> LogSection {
    let files = log_files(dir);
    if files.is_empty() {
        return LogSection::NoLog;
    }
    // Newest last, oldest first: older files are put in front as they are read.
    let mut sorted: Vec<Option<String>> = Vec::new();
    for file in files {
        let mut older = sort_lines(tail_of(&file));
        older.append(&mut sorted);
        sorted = older;
        if sorted.iter().filter(|l| l.is_some()).count() >= max {
            break;
        }
    }
    let mut lines = Vec::new();
    let mut left_out = 0;
    for line in sorted.into_iter().rev() {
        if lines.len() == max {
            break;
        }
        match line {
            Some(line) => lines.push(scrub_line(&line, home)),
            None => left_out += 1,
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
        "Dots problem report\n\
         Made: {made}\n\
         \n\
         == What went wrong ==\n\
         {description}\n\
         \n\
         == This copy of Dots ==\n\
         Version: {version}\n\
         System: {os} {arch}\n\
         Engines in use: {engines}\n\
         Dots: {dots}\n\
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
