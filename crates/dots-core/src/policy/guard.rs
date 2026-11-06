//! The send-and-delete guard ("Always ask before sending or deleting"): which calls send
//! something out of the machine or delete something, so that `Policy::resolve_in` and
//! `Policy::resolve_mcp` ask about them even where a rule, a preset or the default allows them.
//!
//! The classifier is lexical and leans towards asking: a false positive costs the person one
//! click, a false negative lets a dot send or delete unseen. It reads
//! - a shell command (`Bash`, `PowerShell`): each simple command of the line, split at `;`,
//!   `&&`, `||`, `|`, `&`, new lines, brackets and substitutions, with quotes undone, wrappers
//!   (`sudo`, `xargs`, `env`, `timeout` ...) looked through and the strings of `bash -c`,
//!   `cmd /c`, `powershell -Command`, `eval` and `Invoke-Expression` read as commands;
//! - an MCP tool (`mcp__<server>__<tool>`) or an app tool: the tool's name as words
//!   (`send_email`, `chat.postMessage`, `delete-file`).
//!
//! What it cannot see is listed in the design note (docs/superpowers/specs/
//! 2026-11-06-ask-send-delete-design.md): a script the dot wrote and then runs, a command
//! built from variables at run time, a program that sends or deletes on its own.

use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::COMMAND_TOOLS;

/// What a guarded call does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardKind {
    /// Sends, posts, publishes or pushes something out of the machine.
    Send,
    /// Deletes or discards something.
    Delete,
    /// Runs a command Dots cannot read (an encoded PowerShell command, a command built from a
    /// variable), which may send or delete.
    Unclear,
}

/// Why a call is guarded, for the approval card: `summary` follows "<dot> wants to", as in
/// "send an email with Gmail (send_email)" or "delete 3 files: a.txt, b.txt, c.txt".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Guarded {
    pub kind: GuardKind,
    pub summary: String,
}

/// What an approval card shows about a call the guard finds (`Runtime::approval_guard`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalGuard {
    pub kind: GuardKind,
    /// Follows "<dot> wants to".
    pub summary: String,
    /// The dot's "Always ask before sending or deleting" is on: the call always asks, and no
    /// rule can be saved with its approval.
    pub always_asks: bool,
}

/// What the guard finds in a call, or `None` when the call neither sends nor deletes as far as
/// Dots can tell. Dots' own tools and the built-in browser (whose gate asks about its own risky
/// acts) are never guarded here.
pub fn classify(tool: &str, input: &Value) -> Option<Guarded> {
    let tool = tool.trim();
    if COMMAND_TOOLS.contains(&tool) {
        let command = input.get("command").and_then(Value::as_str)?;
        return summarize(shell_hits(command));
    }
    let named = |key: &str| input.get(key).and_then(Value::as_str);
    if tool == crate::apps::APP_POLICY {
        let name = named("tool")?;
        let app = named("app").unwrap_or("an app");
        // An app keeps its data on this machine: its tools can delete, but they send nothing
        // to anyone (a `comment` on a task stays in the app).
        let (kind, phrase) = name_verdict(name).filter(|(k, _)| *k == GuardKind::Delete)?;
        return Some(Guarded {
            kind,
            summary: format!("{phrase} with the app {app} ({name})"),
        });
    }
    let rest = tool.strip_prefix("mcp__")?;
    if tool.starts_with(crate::browser::PREFIX) || rest.starts_with("dots__") {
        return None;
    }
    let (server, name) = rest.split_once("__")?;
    let (kind, phrase) = name_verdict(name)?;
    Some(Guarded {
        kind,
        summary: format!("{phrase} with {} ({name})", display_server(server)),
    })
}

/// `gmail` reads `Gmail`, `google_calendar` reads `Google Calendar`, `claude_ai_Slack` reads
/// `Slack`.
fn display_server(server: &str) -> String {
    let server = server.strip_prefix("claude_ai_").unwrap_or(server);
    server
        .split(['_', '-', '.', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(first) => first.to_uppercase().chain(c).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---- tool names ------------------------------------------------------------------------------

/// A tool name as lower-case words: `chat.postMessage` gives `chat post message`,
/// `send-email` gives `send email`, `HTTPRequest` gives `httprequest`.
pub fn name_words(name: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if !c.is_alphanumeric() {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            prev_lower = false;
            continue;
        }
        if c.is_uppercase() && prev_lower && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        prev_lower = c.is_lowercase() || c.is_ascii_digit();
        current.extend(c.to_lowercase());
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
}

/// Verbs that send on their own.
const SEND_VERBS: &[&str] = &[
    "send",
    "post",
    "reply",
    "respond",
    "publish",
    "share",
    "invite",
    "forward",
    "comment",
    "tweet",
    "retweet",
    "broadcast",
    "push",
    "merge",
    "submit",
    "notify",
    "email",
    "mail",
    "dm",
    "message",
    "react",
    "upload",
    "transfer",
];

/// Verbs that send when what they make is one of `SENT_THINGS`: `create_issue`,
/// `add_comment`, `new_message`.
const MAKE_VERBS: &[&str] = &["create", "add", "new", "write", "compose", "open", "start"];

/// What a `MAKE_VERBS` verb sends: messages, posts, issues and pull requests, events (which
/// invite their attendees), reviews.
const SENT_THINGS: &[&str] = &[
    "message",
    "messages",
    "email",
    "emails",
    "mail",
    "comment",
    "comments",
    "issue",
    "issues",
    "pr",
    "pull",
    "review",
    "event",
    "events",
    "invitation",
    "invite",
    "meeting",
    "chat",
    "dm",
    "tweet",
    "status",
    "reply",
    "discussion",
    "release",
    "gist",
    "webhook",
    "post",
    "thread",
    "announcement",
    "reaction",
];

/// Verbs that delete.
const DELETE_VERBS: &[&str] = &[
    "delete",
    "remove",
    "rm",
    "del",
    "archive",
    "trash",
    "destroy",
    "purge",
    "erase",
    "wipe",
    "drop",
    "unlink",
    "truncate",
    "discard",
    "obliterate",
    "shred",
    "unpublish",
    "terminate",
];

/// Leading verbs of a tool that only reads: `get_post`, `list_comments`, `search_messages`.
/// Such a name is guarded only when it also holds `send` or a `DELETE_VERBS` verb.
const READ_VERBS: &[&str] = &[
    "get", "list", "search", "read", "fetch", "find", "query", "view", "show", "describe", "count",
    "lookup", "download", "check", "preview", "export", "browse", "inspect", "watch",
];

/// The verdict on a tool's name, and the phrase that follows "<dot> wants to" (without "with
/// <server>").
fn name_verdict(name: &str) -> Option<(GuardKind, String)> {
    let words = name_words(name);
    let has = |list: &[&str]| words.iter().any(|w| list.contains(&w.as_str()));
    let first_reads = words
        .first()
        .is_some_and(|w| READ_VERBS.contains(&w.as_str()));
    if has(DELETE_VERBS) {
        let verb = words
            .iter()
            .find(|w| DELETE_VERBS.contains(&w.as_str()))
            .map(String::as_str)
            .unwrap_or("delete");
        let verb = match verb {
            "rm" | "del" | "unlink" | "erase" | "wipe" | "purge" | "obliterate" | "shred" => {
                "delete"
            }
            other => other,
        };
        let object = words
            .iter()
            .skip_while(|w| !DELETE_VERBS.contains(&w.as_str()))
            .skip(1)
            .cloned()
            .collect::<Vec<_>>()
            .join(" ");
        let phrase = if object.is_empty() {
            verb.to_string()
        } else {
            format!("{verb} {object}")
        };
        return Some((GuardKind::Delete, phrase));
    }
    let sends = words.iter().any(|w| w == "send");
    // Drafting is not sending: `create_draft`, `draft_reply`, `update_draft`. Sending one is.
    if words.iter().any(|w| w == "draft" || w == "drafts") && !sends {
        return None;
    }
    if first_reads && !sends {
        return None;
    }
    let makes = has(MAKE_VERBS) && has(SENT_THINGS);
    if !(has(SEND_VERBS) || makes) {
        return None;
    }
    Some((GuardKind::Send, send_phrase(&words)))
}

fn send_phrase(words: &[String]) -> String {
    let has = |w: &str| words.iter().any(|x| x == w);
    let phrase = if has("email") || has("emails") || has("mail") {
        "send an email"
    } else if has("reply") || has("respond") {
        "send a reply"
    } else if has("forward") {
        "forward a message"
    } else if has("comment") || has("comments") {
        "post a comment"
    } else if has("pr") || has("pull") {
        "open or change a pull request"
    } else if has("issue") || has("issues") {
        "create an issue"
    } else if has("event") || has("events") || has("invite") || has("invitation") || has("meeting")
    {
        "send an invitation"
    } else if has("message") || has("messages") || has("chat") || has("dm") {
        "send a message"
    } else if has("publish") || has("release") {
        "publish something"
    } else if has("share") {
        "share something"
    } else if has("post") || has("tweet") {
        "post something"
    } else if has("push") {
        "push changes"
    } else if has("merge") {
        "merge changes"
    } else if has("upload") || has("transfer") {
        "upload something"
    } else {
        "send something"
    };
    phrase.to_string()
}

// ---- shell commands --------------------------------------------------------------------------

/// One thing a command does that the guard asks about.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Hit {
    kind: GuardKind,
    what: String,
}

impl Hit {
    fn send(what: impl Into<String>) -> Hit {
        Hit {
            kind: GuardKind::Send,
            what: what.into(),
        }
    }
    fn delete(what: impl Into<String>) -> Hit {
        Hit {
            kind: GuardKind::Delete,
            what: what.into(),
        }
    }
    fn unclear(what: impl Into<String>) -> Hit {
        Hit {
            kind: GuardKind::Unclear,
            what: what.into(),
        }
    }
}

fn summarize(hits: Vec<Hit>) -> Option<Guarded> {
    let mut unique: Vec<Hit> = Vec::new();
    for hit in hits {
        if !unique.contains(&hit) {
            unique.push(hit);
        }
    }
    let first = unique.first()?;
    let kind = if unique.iter().any(|h| h.kind == GuardKind::Delete) {
        GuardKind::Delete
    } else {
        first.kind
    };
    let mut parts: Vec<&str> = unique.iter().take(3).map(|h| h.what.as_str()).collect();
    let more = unique.len().saturating_sub(3);
    let tail = (more > 0).then(|| format!("{more} more"));
    if let Some(t) = &tail {
        parts.push(t);
    }
    Some(Guarded {
        kind,
        summary: parts.join(", and "),
    })
}

/// How deep `bash -c "powershell -c '...'"` is followed before the guard gives up and asks.
const MAX_DEPTH: usize = 6;

/// What the shell command line `command` does that the guard asks about.
fn shell_hits(command: &str) -> Vec<Hit> {
    let mut hits = Vec::new();
    line_hits(command, 0, &mut hits);
    hits
}

fn line_hits(line: &str, depth: usize, hits: &mut Vec<Hit>) {
    if depth > MAX_DEPTH {
        hits.push(Hit::unclear("run a deeply nested command"));
        return;
    }
    let (commands, substitutions) = split_line(line);
    for sub in substitutions {
        line_hits(&sub, depth + 1, hits);
    }
    let mut db_client = false;
    for words in &commands {
        if words
            .first()
            .is_some_and(|w| DB_CLIENTS.contains(&command_name(w).as_str()))
        {
            db_client = true;
        }
        command_hits(words, depth, hits);
    }
    code_hits(line, db_client, hits);
}

/// Splits a command line into simple commands (each a list of words, quotes undone) and the
/// bodies of its command substitutions (`$(...)`, `` `...` ``), which run as commands of their
/// own. Separators: `;`, `&`, `|`, new lines, `(`, `)`, `{`, `}`. A redirection's target is
/// dropped. Quotes: `'...'` is literal; `"..."` is literal but for its substitutions; a
/// backslash keeps the next character unless it starts a Windows path.
fn split_line(line: &str) -> (Vec<Vec<String>>, Vec<String>) {
    let chars: Vec<char> = line.chars().collect();
    let mut commands: Vec<Vec<String>> = Vec::new();
    let mut subs: Vec<String> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut skip_next = false;
    let mut i = 0;
    let end_word =
        |word: &mut String, in_word: &mut bool, words: &mut Vec<String>, skip: &mut bool| {
            if *in_word {
                if *skip {
                    *skip = false;
                } else {
                    words.push(std::mem::take(word));
                }
                word.clear();
                *in_word = false;
            }
        };
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\'' => {
                in_word = true;
                i += 1;
                while i < chars.len() && chars[i] != '\'' {
                    word.push(chars[i]);
                    i += 1;
                }
            }
            '"' => {
                in_word = true;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    if chars[i] == '\\' && i + 1 < chars.len() && "\"\\$`".contains(chars[i + 1]) {
                        word.push(chars[i + 1]);
                        i += 2;
                        continue;
                    }
                    if chars[i] == '`' && i + 1 < chars.len() && chars[i + 1] == '"' {
                        // PowerShell's escaped quote.
                        word.push('"');
                        i += 2;
                        continue;
                    }
                    if chars[i] == '$' && chars.get(i + 1) == Some(&'(') {
                        let (body, next) = balanced(&chars, i + 1);
                        word.push_str(&format!("$({body})"));
                        subs.push(body);
                        i = next;
                        continue;
                    }
                    if chars[i] == '`' {
                        if let Some(close) = chars[i + 1..].iter().position(|&x| x == '`') {
                            let body: String = chars[i + 1..i + 1 + close].iter().collect();
                            subs.push(body);
                        }
                    }
                    word.push(chars[i]);
                    i += 1;
                }
            }
            '\\' => {
                in_word = true;
                match chars.get(i + 1) {
                    // A line continuation.
                    Some('\n') => i += 1,
                    // `\;` and `\&` are a literal of `find -exec ... \;`: keep them out of the
                    // separators.
                    Some(&n) if ";&|".contains(n) => {
                        word.push(n);
                        i += 1;
                    }
                    _ => word.push('\\'),
                }
            }
            '`' => {
                // A bash substitution, or PowerShell's escape character: read the body as a
                // command too.
                if let Some(close) = chars[i + 1..].iter().position(|&x| x == '`') {
                    let body: String = chars[i + 1..i + 1 + close].iter().collect();
                    subs.push(body);
                }
                in_word = true;
                if let Some(&n) = chars.get(i + 1) {
                    if n != '`' && n != '\n' {
                        word.push(n);
                    }
                    i += 1;
                }
            }
            '$' if chars.get(i + 1) == Some(&'(') => {
                let (body, next) = balanced(&chars, i + 1);
                subs.push(body);
                end_word(&mut word, &mut in_word, &mut words, &mut skip_next);
                i = next;
                continue;
            }
            '<' | '>' => {
                if chars.get(i + 1) == Some(&'(') {
                    let (body, next) = balanced(&chars, i + 1);
                    subs.push(body);
                    i = next;
                    continue;
                }
                // `2>` and `&>`: the number belongs to the operator.
                if in_word && word.chars().all(|d| d.is_ascii_digit()) {
                    word.clear();
                    in_word = false;
                }
                end_word(&mut word, &mut in_word, &mut words, &mut skip_next);
                while matches!(chars.get(i + 1), Some('>' | '<' | '&')) {
                    i += 1;
                }
                skip_next = true;
            }
            '#' if !in_word => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            ';' | '&' | '|' | '\n' | '\r' | '(' | ')' | '{' | '}' => {
                end_word(&mut word, &mut in_word, &mut words, &mut skip_next);
                skip_next = false;
                if !words.is_empty() {
                    commands.push(std::mem::take(&mut words));
                }
            }
            c if c.is_whitespace() => {
                end_word(&mut word, &mut in_word, &mut words, &mut skip_next);
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
        i += 1;
    }
    end_word(&mut word, &mut in_word, &mut words, &mut skip_next);
    if !words.is_empty() {
        commands.push(words);
    }
    (commands, subs)
}

/// The body of the bracket group that opens at `chars[open]` (`(`), and the index after its
/// close. An unclosed group runs to the end of the line.
fn balanced(chars: &[char], open: usize) -> (String, usize) {
    let mut depth = 0usize;
    let mut i = open;
    while i < chars.len() {
        match chars[i] {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return (chars[open + 1..i].iter().collect(), i + 1);
                }
            }
            _ => {}
        }
        i += 1;
    }
    (
        chars[(open + 1).min(chars.len())..].iter().collect(),
        chars.len(),
    )
}

/// A command word as a name: lower case, without its folder, leading backslashes, PowerShell's
/// call operator and `.exe`/`.cmd`/`.bat`/`.com`.
fn command_name(word: &str) -> String {
    let w = word.trim_start_matches(['\\', '&', '.']).trim();
    let base = w.rsplit(['/', '\\']).next().unwrap_or(w).to_lowercase();
    for ext in [".exe", ".cmd", ".bat", ".com"] {
        if let Some(stem) = base.strip_suffix(ext) {
            return stem.to_string();
        }
    }
    base
}

fn is_flag(word: &str) -> bool {
    word.starts_with('-') || (word.starts_with('/') && word.len() <= 3 && !word[1..].contains('/'))
}

/// The non-flag words after the command name.
fn operands(words: &[String]) -> Vec<&str> {
    words
        .iter()
        .skip(1)
        .map(String::as_str)
        .filter(|w| !w.is_empty() && !is_flag(w))
        .collect()
}

/// "a.txt", "2 files: a.txt, b.txt", or `none` when there is nothing to name.
fn listed(targets: &[&str], noun: &str, none: &str) -> String {
    match targets {
        [] => none.to_string(),
        [one] => (*one).to_string(),
        many => {
            let shown: Vec<&str> = many.iter().take(5).copied().collect();
            let rest = many.len().saturating_sub(5);
            let extra = if rest > 0 {
                format!(" and {rest} more")
            } else {
                String::new()
            };
            format!("{} {noun}: {}{extra}", many.len(), shown.join(", "))
        }
    }
}

/// Commands that run the command that follows them (after their own options): every later
/// word is tried as the start of a command.
const WRAPPERS: &[&str] = &[
    "sudo",
    "doas",
    "env",
    "nohup",
    "time",
    "nice",
    "ionice",
    "timeout",
    "xargs",
    "stdbuf",
    "chroot",
    "watch",
    "command",
    "builtin",
    "exec",
    "start",
    "call",
    "runas",
    "strace",
    "unbuffer",
    "parallel",
    "npx",
    "bunx",
    "pnpx",
    "dlx",
    "wsl",
    "busybox",
    "caffeinate",
    "chronic",
    "flock",
    "setsid",
    "torsocks",
    "proxychains",
    "start-process",
    "saps",
    "invoke-command",
    "icm",
    "start-job",
    "sajb",
    "foreach-object",
    "%",
    "foreach",
    "where-object",
    "measure-command",
    "su",
    "ssh",
    "script",
];

/// Shells whose `-c` string is a command line.
const SHELLS: &[&str] = &[
    "sh", "bash", "zsh", "dash", "ksh", "fish", "ash", "csh", "tcsh",
];

/// Command-line database clients: their SQL is scanned for deletes (`code_hits`).
const DB_CLIENTS: &[&str] = &[
    "sqlite3",
    "sqlite",
    "psql",
    "mysql",
    "mariadb",
    "sqlcmd",
    "duckdb",
    "mongosh",
    "mongo",
    "clickhouse",
    "clickhouse-client",
    "cockroach",
    "bq",
    "snowsql",
    "sqlplus",
    "invoke-sqlcmd",
    "redis-cli",
    "cqlsh",
    "osqlcmd",
    "usql",
    "litecli",
    "pgcli",
    "mycli",
];

/// Commands that delete their operands.
const DELETE_COMMANDS: &[&str] = &[
    "rm",
    "unlink",
    "shred",
    "srm",
    "trash",
    "trash-put",
    "rimraf",
    "del-cli",
    "del",
    "erase",
    "remove-item",
    "ri",
    "sdelete",
    "wipe",
    "truncate",
    "clear-content",
    "clc",
    "clear-item",
    "cli",
    "clear-recyclebin",
];

/// Commands that delete folders.
const RMDIR_COMMANDS: &[&str] = &["rmdir", "rd"];

/// Commands that destroy a whole disk or volume.
const DISK_COMMANDS: &[&str] = &["mkfs", "diskpart", "format", "format-volume", "clear-disk"];

/// Command-line mail senders.
const MAIL_COMMANDS: &[&str] = &[
    "sendmail",
    "mail",
    "mailx",
    "mutt",
    "msmtp",
    "swaks",
    "ssmtp",
    "send-mailmessage",
    "neomutt",
];

/// Tools with subcommands, whose subcommand words say whether they delete or publish.
const SUBCOMMAND_CLIS: &[&str] = &[
    "aws",
    "az",
    "gcloud",
    "gsutil",
    "kubectl",
    "oc",
    "helm",
    "docker",
    "podman",
    "terraform",
    "tofu",
    "pulumi",
    "heroku",
    "vercel",
    "netlify",
    "fly",
    "flyctl",
    "firebase",
    "supabase",
    "wrangler",
    "doctl",
    "redis-cli",
    "npm",
    "pnpm",
    "yarn",
    "bun",
    "cargo",
    "dotnet",
    "nuget",
    "twine",
    "gem",
    "poetry",
    "flit",
    "vsce",
    "ovsx",
    "s3cmd",
    "rclone",
    "mc",
    "hcloud",
    "linode-cli",
    "railway",
    "render",
    "kind",
    "minikube",
    "nomad",
    "consul",
    "vault",
    "deno",
    "gsutil",
    "azcopy",
    "glab",
    "tea",
    "hub",
    "jira",
    "slack",
    "discord",
    "stripe",
    "twilio",
    "sfdx",
    "sf",
    "aws-vault",
];

const SUBCOMMAND_DELETES: &[&str] = &[
    "delete",
    "del",
    "rm",
    "rmi",
    "rb",
    "remove",
    "destroy",
    "purge",
    "prune",
    "drop",
    "flushall",
    "flushdb",
    "unpublish",
    "uninstall",
    "rmdir",
    "rmdirs",
    "deletefile",
    "cleanup",
    "unlink",
    "erase",
    "wipe",
    "yank",
    "trash",
];

const SUBCOMMAND_SENDS: &[&str] = &[
    "publish", "push", "upload", "send", "post", "release", "comment", "reply", "share", "invite",
];

/// What one simple command does that the guard asks about.
fn command_hits(words: &[String], depth: usize, hits: &mut Vec<Hit>) {
    // `VAR=value cmd`, PowerShell's `$x = cmd` and `$x=cmd`.
    let mut owned: Vec<String> = words.to_vec();
    loop {
        let Some(w) = owned.first().cloned() else {
            return;
        };
        // Shell keywords that come before a command: `if rm x`, `then rm x`, `do rm "$f"`.
        if ["if", "then", "else", "elif", "do", "while", "until", "!"].contains(&w.as_str()) {
            owned.remove(0);
            continue;
        }
        if w.starts_with('$') && owned.get(1).is_some_and(|n| n == "=") {
            owned.drain(..2);
            continue;
        }
        if let Some((name, rest)) = w.split_once('=') {
            let plain = !name.is_empty()
                && !name.starts_with('-')
                && name
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '_' || c == '$');
            if plain {
                if name.starts_with('$') && !rest.is_empty() {
                    owned[0] = rest.to_string();
                } else {
                    owned.remove(0);
                }
                continue;
            }
        }
        break;
    }
    let words = owned.as_slice();
    let first = &words[0];
    let name = command_name(first);
    let ops = operands(words);
    let has_flag = |flags: &[&str]| {
        words
            .iter()
            .skip(1)
            .any(|w| flags.iter().any(|f| w.eq_ignore_ascii_case(f)))
    };
    if WRAPPERS.contains(&name.as_str()) {
        // Every later word may be the command: `sudo -u bob rm x`, `xargs -0 rm`, `timeout 5 rm`.
        for i in 1..words.len() {
            command_hits(&words[i..], depth, hits);
        }
        // `ssh host "rm -rf x"`, `wsl -e bash -c "..."`, `Start-Process rm -ArgumentList x`.
        for w in &words[1..] {
            if w.contains(' ') {
                line_hits(w, depth + 1, hits);
            }
        }
        return;
    }
    if SHELLS.contains(&name.as_str()) {
        let mut it = words.iter().skip(1);
        while let Some(w) = it.next() {
            let is_c =
                w == "-c" || (w.starts_with('-') && !w.starts_with("--") && w.ends_with('c'));
            if is_c {
                if let Some(script) = it.next() {
                    line_hits(script, depth + 1, hits);
                }
                break;
            }
        }
        return;
    }
    if name == "cmd" {
        if let Some(pos) = words.iter().position(|w| {
            w.get(..2)
                .is_some_and(|h| ["/c", "/k", "/r"].contains(&h.to_lowercase().as_str()))
        }) {
            let head = &words[pos][2..];
            let mut rest: Vec<&str> = Vec::new();
            if !head.is_empty() {
                rest.push(head);
            }
            rest.extend(words[pos + 1..].iter().map(String::as_str));
            line_hits(&rest.join(" "), depth + 1, hits);
        }
        return;
    }
    if name == "powershell" || name == "pwsh" {
        powershell_hits(words, depth, hits);
        return;
    }
    if name == "eval" || name == "iex" || name == "invoke-expression" {
        let rest: Vec<&str> = words[1..].iter().map(String::as_str).collect();
        if rest.iter().any(|w| is_variable(w)) {
            hits.push(Hit::unclear(format!(
                "run a command built at run time ({})",
                words.join(" ")
            )));
        }
        line_hits(&rest.join(" "), depth + 1, hits);
        return;
    }
    if name == "find" {
        if has_flag(&["-delete"]) {
            hits.push(Hit::delete("delete the files find matches"));
        }
        for (i, w) in words.iter().enumerate() {
            if ["-exec", "-execdir", "-ok", "-okdir"].contains(&w.as_str()) {
                let tail: Vec<String> = words[i + 1..]
                    .iter()
                    .take_while(|t| *t != ";" && *t != "+")
                    .cloned()
                    .collect();
                command_hits(&tail, depth, hits);
            }
        }
        return;
    }
    if name == "git" {
        git_hits(words, hits);
        return;
    }
    if name == "gh" {
        gh_hits(words, hits);
        return;
    }
    if name == "curl"
        || name == "wget"
        || name == "invoke-webrequest"
        || name == "iwr"
        || name == "invoke-restmethod"
        || name == "irm"
    {
        web_hits(&name, words, hits);
        return;
    }
    if ["http", "https", "xh", "xhs", "httpie"].contains(&name.as_str()) {
        let method = ops.first().map(|m| m.to_uppercase());
        match method.as_deref() {
            Some("DELETE") => hits.push(Hit::delete(format!(
                "send a DELETE request ({})",
                words.join(" ")
            ))),
            Some("POST" | "PUT" | "PATCH") => hits.push(Hit::send(format!(
                "send data with {name} ({})",
                words.join(" ")
            ))),
            _ => {}
        }
        return;
    }
    if DELETE_COMMANDS.contains(&name.as_str()) {
        let targets: Vec<&str> = ops.clone();
        hits.push(Hit::delete(format!(
            "delete {}",
            listed(&targets, "files", "files")
        )));
        return;
    }
    if RMDIR_COMMANDS.contains(&name.as_str()) {
        hits.push(Hit::delete(format!(
            "delete {}",
            listed(&ops, "folders", "a folder")
        )));
        return;
    }
    if DISK_COMMANDS.contains(&name.as_str()) || name.starts_with("mkfs.") {
        hits.push(Hit::delete(format!("erase a disk ({})", words.join(" "))));
        return;
    }
    if name == "dd" && words.iter().any(|w| w.starts_with("of=")) {
        hits.push(Hit::delete(format!(
            "overwrite with dd ({})",
            words.join(" ")
        )));
        return;
    }
    if name == "cipher" && words.iter().any(|w| w.to_lowercase().starts_with("/w")) {
        hits.push(Hit::delete("wipe free space with cipher"));
        return;
    }
    if name == "gio" && ops.first().is_some_and(|o| *o == "trash" || *o == "remove") {
        hits.push(Hit::delete(format!(
            "delete {}",
            listed(&ops[1..], "files", "files")
        )));
        return;
    }
    if MAIL_COMMANDS.contains(&name.as_str()) {
        hits.push(Hit::send(format!("send an email ({})", words.join(" "))));
        return;
    }
    // PowerShell verbs: Remove-*, Clear-*, Send-*, Publish-*.
    if let Some((verb, _)) = name.split_once('-') {
        match verb {
            "remove" => {
                hits.push(Hit::delete(format!(
                    "delete with {first} ({})",
                    words.join(" ")
                )));
                return;
            }
            "send" | "publish" | "submit" => {
                hits.push(Hit::send(format!(
                    "{verb} with {first} ({})",
                    words.join(" ")
                )));
                return;
            }
            _ => {}
        }
    }
    if name == "rsync" {
        if words
            .iter()
            .any(|w| w.starts_with("--delete") || w == "--remove-source-files")
        {
            hits.push(Hit::delete(format!(
                "delete files with rsync ({})",
                words.join(" ")
            )));
        } else if ops.iter().skip(1).any(|o| is_remote(o)) {
            hits.push(Hit::send(format!(
                "copy files to another computer ({})",
                words.join(" ")
            )));
        }
        return;
    }
    if name == "scp" || name == "sftp" {
        if ops.last().is_some_and(|o| is_remote(o)) {
            hits.push(Hit::send(format!(
                "copy files to another computer ({})",
                words.join(" ")
            )));
        }
        return;
    }
    if SUBCOMMAND_CLIS.contains(&name.as_str()) {
        let subs: Vec<String> = ops.iter().take(4).map(|o| o.to_lowercase()).collect();
        let deletes = subs.iter().any(|s| {
            SUBCOMMAND_DELETES.contains(&s.as_str())
                || s.starts_with("delete-")
                || s.starts_with("remove-")
                || s.starts_with("terminate-")
                || s.starts_with("destroy")
                || s.ends_with(":destroy")
                || s.ends_with(":delete")
                || s.ends_with(":remove")
        });
        // `npm uninstall` and friends change the project's packages: only a release manager's
        // `uninstall` (helm) deletes something that runs.
        let package_manager =
            ["npm", "pnpm", "yarn", "bun", "cargo", "dotnet", "poetry"].contains(&name.as_str());
        let only_uninstall = subs
            .iter()
            .filter(|s| SUBCOMMAND_DELETES.contains(&s.as_str()))
            .all(|s| s == "uninstall" || s == "remove" || s == "rm");
        if deletes && !(package_manager && only_uninstall) {
            hits.push(Hit::delete(format!(
                "delete with {name} ({})",
                words.join(" ")
            )));
            return;
        }
        let sends = subs.iter().any(|s| SUBCOMMAND_SENDS.contains(&s.as_str()));
        if sends {
            hits.push(Hit::send(format!(
                "publish with {name} ({})",
                words.join(" ")
            )));
        }
    }
}

fn is_variable(word: &str) -> bool {
    let mut c = word.chars();
    c.next() == Some('$')
        && c.next()
            .is_some_and(|n| n.is_ascii_alphabetic() || n == '_' || n == '{')
}

/// `host:path` or `user@host:path`, not a Windows drive (`C:\x`).
fn is_remote(operand: &str) -> bool {
    match operand.split_once(':') {
        Some((host, _)) => host.len() > 1 && !host.contains(['/', '\\']),
        None => false,
    }
}

fn powershell_hits(words: &[String], depth: usize, hits: &mut Vec<Hit>) {
    // Options that take a value, and are not the command.
    const VALUED: &[&str] = &[
        "-executionpolicy",
        "-ep",
        "-ex",
        "-windowstyle",
        "-w",
        "-workingdirectory",
        "-wd",
        "-configurationname",
        "-settingsfile",
        "-inputformat",
        "-outputformat",
        "-of",
        "-if",
        "-psconsolefile",
        "-version",
        "-v",
    ];
    let lower: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let mut i = 1;
    while i < words.len() {
        let w = lower[i].as_str();
        let prefix_of = |full: &str, min: usize| w.len() >= min && full.starts_with(w);
        if VALUED.contains(&w) {
            i += 2;
            continue;
        }
        if w == "-e" || w == "-ec" || prefix_of("-encodedcommand", 3) {
            let decoded = words.get(i + 1).and_then(|b| decode_encoded(b));
            match decoded {
                Some(script) => line_hits(&script, depth + 1, hits),
                None => hits.push(Hit::unclear(
                    "run an encoded PowerShell command that Dots cannot read",
                )),
            }
            return;
        }
        if prefix_of("-command", 2) {
            let rest: Vec<&str> = words[i + 1..].iter().map(String::as_str).collect();
            line_hits(&rest.join(" "), depth + 1, hits);
            return;
        }
        if w.starts_with('-') && prefix_of("-file", 2) {
            // A script file: the guard cannot read it (a known gap).
            return;
        }
        if w.starts_with('-') {
            i += 1;
            continue;
        }
        // The first word that is not an option starts the command.
        let rest: Vec<&str> = words[i..].iter().map(String::as_str).collect();
        line_hits(&rest.join(" "), depth + 1, hits);
        return;
    }
}

/// PowerShell's `-EncodedCommand`: base64 of UTF-16LE text.
fn decode_encoded(b64: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .ok()?;
    if bytes.len() % 2 != 0 {
        return None;
    }
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16(&units).ok()
}

fn git_hits(words: &[String], hits: &mut Vec<Hit>) {
    // Skip git's own options: `-C dir`, `-c key=value`, `--git-dir=...`, `--no-pager`.
    let mut i = 1;
    while i < words.len() && words[i].starts_with('-') {
        i += if ["-C", "-c", "--git-dir", "--work-tree", "--namespace"].contains(&words[i].as_str())
        {
            2
        } else {
            1
        };
    }
    let Some(sub) = words.get(i).map(|s| s.to_lowercase()) else {
        return;
    };
    let args: Vec<&str> = words[i + 1..].iter().map(String::as_str).collect();
    let has = |flags: &[&str]| args.iter().any(|a| flags.contains(a));
    if has(&["--help", "-h"]) {
        return;
    }
    let shown = words.join(" ");
    match sub.as_str() {
        "push" => {
            let destructive = args.iter().any(|a| {
                [
                    "-f",
                    "--force",
                    "--delete",
                    "-d",
                    "--mirror",
                    "--prune",
                    "--force-if-includes",
                ]
                .contains(a)
                    || a.starts_with("--force-with-lease")
                    || (a.starts_with('-')
                        && !a.starts_with("--")
                        && (a.contains('f') || a.contains('d')))
                    || a.starts_with('+')
                    || a.starts_with(':')
            });
            if destructive {
                hits.push(Hit::delete(format!(
                    "force-push or delete on a remote with git, which can lose commits there ({shown})"
                )));
            } else {
                hits.push(Hit::send(format!("push commits with git ({shown})")));
            }
        }
        "send-email" | "imap-send" => {
            hits.push(Hit::send(format!("send an email with git ({shown})")))
        }
        "clean" => hits.push(Hit::delete(format!(
            "delete untracked files with git clean ({shown})"
        ))),
        "reset" if has(&["--hard", "--merge", "--keep"]) => hits.push(Hit::delete(format!(
            "discard changes with git reset ({shown})"
        ))),
        "branch"
            if args.iter().any(|a| {
                *a == "--delete"
                    || (a.starts_with('-')
                        && !a.starts_with("--")
                        && (a.contains('d') || a.contains('D')))
            }) =>
        {
            hits.push(Hit::delete(format!("delete a branch with git ({shown})")))
        }
        "tag" if has(&["-d", "--delete"]) => {
            hits.push(Hit::delete(format!("delete a tag with git ({shown})")))
        }
        "rm" => {
            let targets: Vec<&str> = args.iter().copied().filter(|a| !is_flag(a)).collect();
            hits.push(Hit::delete(format!(
                "delete {} with git rm",
                listed(&targets, "files", "files")
            )));
        }
        "stash" if args.first().is_some_and(|a| *a == "drop" || *a == "clear") => {
            hits.push(Hit::delete(format!("throw away stashed changes ({shown})")))
        }
        "checkout" if has(&["--", ".", "-f", "--force"]) => hits.push(Hit::delete(format!(
            "discard changes with git checkout ({shown})"
        ))),
        "restore" if !has(&["--staged", "-S"]) || has(&["--worktree", "-W"]) => hits.push(
            Hit::delete(format!("discard changes with git restore ({shown})")),
        ),
        "switch" if has(&["-f", "--force", "--discard-changes"]) => hits.push(Hit::delete(
            format!("discard changes with git switch ({shown})"),
        )),
        "worktree" | "remote" | "submodule" | "notes" | "lfs"
            if args
                .first()
                .is_some_and(|a| ["remove", "rm", "prune", "deinit", "delete"].contains(a)) =>
        {
            hits.push(Hit::delete(format!("delete with git ({shown})")))
        }
        "reflog"
            if args
                .first()
                .is_some_and(|a| *a == "expire" || *a == "delete") =>
        {
            hits.push(Hit::delete(format!("delete history with git ({shown})")))
        }
        "update-ref" if has(&["-d"]) => {
            hits.push(Hit::delete(format!("delete a ref with git ({shown})")))
        }
        "filter-branch" | "filter-repo" => {
            hits.push(Hit::delete(format!("rewrite history with git ({shown})")))
        }
        "gc" | "prune" if args.iter().any(|a| a.starts_with("--prune")) || sub == "prune" => hits
            .push(Hit::delete(format!(
                "delete unreachable commits with git ({shown})"
            ))),
        _ => {}
    }
}

fn gh_hits(words: &[String], hits: &mut Vec<Hit>) {
    let ops: Vec<String> = operands(words).iter().map(|o| o.to_lowercase()).collect();
    let shown = words.join(" ");
    let group = ops.first().map(String::as_str).unwrap_or("");
    let action = ops.get(1).map(String::as_str).unwrap_or("");
    if ops
        .iter()
        .take(3)
        .any(|o| ["delete", "remove", "rm", "archive", "delete-asset"].contains(&o.as_str()))
    {
        hits.push(Hit::delete(format!("delete on GitHub ({shown})")));
        return;
    }
    if group == "api" {
        let mut method: Option<String> = None;
        let mut fields = false;
        let mut it = words.iter().skip(1);
        while let Some(w) = it.next() {
            if w == "-X" || w == "--method" {
                method = it.next().map(|m| m.to_uppercase());
            } else if let Some(m) = w.strip_prefix("--method=").or_else(|| w.strip_prefix("-X")) {
                if !m.is_empty() {
                    method = Some(m.to_uppercase());
                }
            } else if ["-f", "-F", "--field", "--raw-field", "--input"].contains(&w.as_str())
                || w.starts_with("--field=")
                || w.starts_with("--raw-field=")
            {
                fields = true;
            }
        }
        match method.as_deref() {
            Some("DELETE") => hits.push(Hit::delete(format!("delete on GitHub ({shown})"))),
            Some("POST" | "PUT" | "PATCH") => {
                hits.push(Hit::send(format!("change something on GitHub ({shown})")))
            }
            Some(_) => {}
            None if fields => hits.push(Hit::send(format!("change something on GitHub ({shown})"))),
            None => {}
        }
        return;
    }
    let what = match (group, action) {
        ("pr", "create" | "new") => "open a pull request on GitHub",
        ("pr", "comment") => "comment on a pull request on GitHub",
        ("pr", "merge") => "merge a pull request on GitHub",
        ("pr", "review") => "post a review on GitHub",
        ("pr", "edit" | "close" | "reopen" | "ready") => "change a pull request on GitHub",
        ("issue", "create" | "new") => "create an issue on GitHub",
        ("issue", "comment") => "comment on an issue on GitHub",
        ("issue", "edit" | "close" | "reopen" | "transfer") => "change an issue on GitHub",
        ("release", "create" | "upload" | "edit") => "publish a release on GitHub",
        ("repo", "create" | "fork" | "edit" | "rename") => {
            "create or change a repository on GitHub"
        }
        ("gist", "create" | "new" | "edit") => "publish a gist on GitHub",
        ("secret" | "variable", "set") => "change a secret on GitHub",
        ("workflow", "run") | ("run", "rerun") => "start a workflow on GitHub",
        ("discussion", _) if action != "list" && action != "view" => "post a discussion on GitHub",
        _ => return,
    };
    hits.push(Hit::send(format!("{what} ({shown})")));
}

fn web_hits(name: &str, words: &[String], hits: &mut Vec<Hit>) {
    let mut method: Option<String> = None;
    let mut body = false;
    let mut url: Option<&str> = None;
    let mut it = words.iter().skip(1).peekable();
    while let Some(w) = it.next() {
        let lw = w.to_lowercase();
        // curl and wget.
        if w == "-X" || lw == "--request" {
            method = it.next().map(|m| m.to_uppercase());
        } else if let Some(m) = w.strip_prefix("-X").filter(|m| !m.is_empty()) {
            method = Some(m.to_uppercase());
        } else if let Some(m) = lw
            .strip_prefix("--request=")
            .or_else(|| lw.strip_prefix("--method="))
        {
            method = Some(m.to_uppercase());
        } else if ["-d", "-F", "-T"].contains(&w.as_str())
            || [
                "--json",
                "--form",
                "--upload-file",
                "--post-data",
                "--post-file",
                "--body-data",
                "--body-file",
            ]
            .contains(&lw.as_str())
            || w.starts_with("-d") && w.len() > 2 && !w.starts_with("--")
            || lw.starts_with("--data")
            || lw.starts_with("--form")
            || lw.starts_with("--post-")
            || lw.starts_with("--json")
        {
            body = true;
        } else if lw.starts_with('-')
            && lw.len() >= 3
            && "-method".starts_with(lw.split(':').next().unwrap_or(&lw))
        {
            // PowerShell's `-Method Post`, `-Method:Post`, `-me post`.
            match lw.split_once(':') {
                Some((_, m)) if !m.is_empty() => method = Some(m.to_uppercase()),
                _ => method = it.next().map(|m| m.to_uppercase()),
            }
        } else if ["-body", "-infile", "-form"].contains(&lw.as_str()) {
            body = true;
        } else if url.is_none() && (lw.contains("://") || lw.starts_with("www.")) {
            url = Some(w.as_str());
        } else if lw == "-uri" {
            url = it.peek().map(|u| u.as_str());
        }
    }
    let host = url
        .map(|u| {
            let rest = u.split_once("://").map(|(_, r)| r).unwrap_or(u);
            rest.split(['/', '?', '#'])
                .next()
                .unwrap_or(rest)
                .to_string()
        })
        .unwrap_or_else(|| "a web address".to_string());
    match method.as_deref() {
        Some("DELETE") => hits.push(Hit::delete(format!(
            "send a DELETE request to {host} with {name}"
        ))),
        Some("POST" | "PUT" | "PATCH") => {
            hits.push(Hit::send(format!("send data to {host} with {name}")))
        }
        Some("GET" | "HEAD" | "OPTIONS") => {}
        Some(other) => hits.push(Hit::send(format!(
            "send a {other} request to {host} with {name}"
        ))),
        None if body => hits.push(Hit::send(format!("send data to {host} with {name}"))),
        None => {}
    }
}

/// Code inside a command line that deletes or sends: `python -c "shutil.rmtree(...)"`,
/// `node -e "fs.rmSync(...)"`, `[IO.File]::Delete(...)`. With `db_client`, the line's SQL too
/// (`DELETE FROM`, `DROP TABLE`, `TRUNCATE`).
fn code_hits(line: &str, db_client: bool, hits: &mut Vec<Hit>) {
    let lower: String = line
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    const DELETES: &[&str] = &[
        "shutil.rmtree",
        "os.remove(",
        "os.unlink(",
        "os.rmdir(",
        "os.removedirs(",
        ".unlink(",
        ".rmdir(",
        "fs.rm(",
        ".rmsync(",
        ".unlinksync(",
        ".rmdirsync(",
        "fs.unlink(",
        "fs.rmdir(",
        "fs.promises.rm",
        ".rm(",
        "rimraf",
        "::delete(",
        "]::delete",
        ".delete()",
        "file.delete(",
        "directory.delete(",
        "fileutils.rm",
        "file.unlink",
        "requests.delete(",
        "deletemany(",
        "deleteone(",
        "dropdatabase(",
        ".drop()",
    ];
    const SENDS: &[&str] = &[
        "requests.post(",
        "requests.put(",
        "requests.patch(",
        "httpx.post(",
        "httpx.put(",
        "smtplib",
        "sendmail(",
        "send_message(",
        "net.mail.smtpclient",
        "uploadfile(",
        "method: 'post'",
        "method: \"post\"",
        "method:'post'",
        "method:\"post\"",
        "urllib.request.urlopen(",
    ];
    if let Some(found) = DELETES.iter().find(|p| lower.contains(*p)) {
        hits.push(Hit::delete(format!(
            "run code that deletes ({})",
            found.trim_end_matches('(')
        )));
    }
    if let Some(found) = SENDS.iter().find(|p| lower.contains(*p)) {
        hits.push(Hit::send(format!(
            "run code that sends ({})",
            found.trim_end_matches('(')
        )));
    }
    if db_client {
        const SQL: &[&str] = &[
            "delete from",
            "drop table",
            "drop database",
            "drop schema",
            "drop view",
            "drop index",
            "drop collection",
            "truncate ",
            "flushall",
            "flushdb",
            "del ",
        ];
        if let Some(found) = SQL.iter().find(|p| lower.contains(*p)) {
            hits.push(Hit::delete(format!(
                "delete data with SQL ({})",
                found.trim()
            )));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_words_split_case_and_punctuation() {
        assert_eq!(name_words("chat.postMessage"), ["chat", "post", "message"]);
        assert_eq!(name_words("send-email"), ["send", "email"]);
        assert_eq!(name_words("GMAIL_SEND_EMAIL"), ["gmail", "send", "email"]);
        assert_eq!(name_words("deleteFile2"), ["delete", "file2"]);
    }

    #[test]
    fn a_split_line_undoes_quotes_and_finds_substitutions() {
        let (commands, subs) = split_line(r#"echo "a; b" && r'm' x | wc; echo $(rm y)"#);
        assert_eq!(commands[0], ["echo", "a; b"]);
        assert_eq!(commands[1], ["rm", "x"]);
        assert_eq!(commands[2], ["wc"]);
        assert_eq!(subs, ["rm y"]);
    }

    #[test]
    fn a_redirection_target_is_not_a_command() {
        let (commands, _) = split_line("ls > rm 2>&1");
        assert_eq!(commands, vec![vec!["ls".to_string()]]);
    }

    #[test]
    fn an_encoded_command_is_read() {
        let script = "Remove-Item C:\\x -Recurse";
        let utf16: Vec<u8> = script
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        let b64 = base64::engine::general_purpose::STANDARD.encode(utf16);
        assert_eq!(decode_encoded(&b64).as_deref(), Some(script));
    }
}
