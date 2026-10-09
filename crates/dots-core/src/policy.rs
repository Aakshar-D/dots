//! Per-dot tool policy.
//!
//! Allow matching is strict: an allow rule only matches inputs it can fully understand
//! (plain commands, clean relative or workspace-contained paths, plain hostnames).
//! Deny/ask matching is best-effort and fails closed where it can, but shell escapes,
//! variables and aliases can still evade command rules. The per-run worktree plus the
//! CLI-enforced deny lists (`Policy::cli_lists`) are the actual isolation boundary.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Error, Result};

str_enum!(Action { Allow => "allow", Ask => "ask", Deny => "deny" });
str_enum!(Preset {
    ReadOnly => "read-only",
    Sandboxed => "sandboxed",
    Trusted => "trusted",
    Custom => "custom",
});

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rule {
    pub tool: String,
    pub action: Action,
}

impl Rule {
    pub fn new(tool: &str, action: Action) -> Self {
        Self {
            tool: tool.to_string(),
            action,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub preset: Preset,
    pub rules: Vec<Rule>,
    pub default: Action,
}

const READ_TOOLS: &[&str] = &["Read", "LS", "Glob", "Grep"];
const COMMAND_TOOLS: &[&str] = &["Bash", "PowerShell"];
const FILE_TOOLS: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "NotebookEdit",
    "Glob",
    "Grep",
    "LS",
];

impl Policy {
    pub fn preset(preset: Preset) -> Policy {
        let mut rules: Vec<Rule> = Vec::new();
        let default = match preset {
            Preset::ReadOnly => {
                rules.extend(READ_TOOLS.iter().map(|t| Rule::new(t, Action::Allow)));
                for t in ["Write", "Edit", "NotebookEdit", "Bash", "PowerShell"] {
                    rules.push(Rule::new(t, Action::Deny));
                }
                Action::Deny
            }
            // `Custom` starts from the Sandboxed rules for the user to edit.
            Preset::Sandboxed | Preset::Custom => {
                rules.extend(READ_TOOLS.iter().map(|t| Rule::new(t, Action::Allow)));
                rules.push(Rule::new("Write(./**)", Action::Allow));
                rules.push(Rule::new("Edit(./**)", Action::Allow));
                for git in ["status", "diff", "add", "commit"] {
                    rules.push(Rule::new(&format!("Bash(git {git}:*)"), Action::Allow));
                }
                for tool in ["Write", "Edit"] {
                    rules.push(Rule::new(&format!("{tool}(**/.git/**)"), Action::Deny));
                    rules.push(Rule::new(&format!("{tool}(**/.git)"), Action::Deny));
                }
                Action::Ask
            }
            Preset::Trusted => {
                rules.push(Rule::new("Bash(git push:*)", Action::Ask));
                rules.push(Rule::new("PowerShell(git push:*)", Action::Ask));
                rules.push(Rule::new("mcp__*", Action::Ask));
                Action::Allow
            }
        };
        Policy {
            preset,
            rules,
            default,
        }
    }

    /// Deny rules win over allow rules, which win over ask rules; otherwise `default`.
    pub fn resolve(&self, tool: &str, input: &Value) -> Action {
        self.resolve_in(tool, input, None)
    }

    /// Like `resolve`, but absolute file paths inside `workspace` are treated as relative to it.
    pub fn resolve_in(&self, tool: &str, input: &Value, workspace: Option<&Path>) -> Action {
        let ws = workspace.map(|w| lexical_normalize(&w.to_string_lossy()));
        let ws = ws.as_ref().filter(|(abs, _)| *abs).map(|(_, t)| t.as_str());
        for action in [Action::Deny, Action::Allow, Action::Ask] {
            if self
                .rules
                .iter()
                .any(|r| r.action == action && pattern_matches(&r.tool, tool, input, action, ws))
            {
                return action;
            }
        }
        self.default
    }

    /// (allow patterns, deny patterns) for `--allowedTools` / `--disallowedTools`.
    pub fn cli_lists(&self) -> (Vec<String>, Vec<String>) {
        let pick = |a: Action| -> Vec<String> {
            self.rules
                .iter()
                .filter(|r| r.action == a)
                .map(|r| r.tool.trim().to_string())
                .collect()
        };
        (pick(Action::Allow), pick(Action::Deny))
    }

    pub fn validate(&self) -> Result<()> {
        for r in &self.rules {
            let invalid = |e: &str| Error::Invalid(format!("policy rule {:?}: {e}", r.tool));
            let pattern = parse_pattern(&r.tool).map_err(|e| invalid(&e))?;
            let name = match pattern {
                Pattern::Any => "",
                Pattern::Exact(n) | Pattern::Prefix(n) => n,
                Pattern::WithSpec { tool, .. } => tool,
            };
            if name.chars().any(char::is_whitespace) {
                return Err(invalid("tool name must not contain whitespace"));
            }
            if let Pattern::WithSpec { tool, spec } = pattern {
                if COMMAND_TOOLS.contains(&tool) {
                    let body = spec.strip_suffix(":*");
                    if body.unwrap_or(spec).contains('*') {
                        return Err(invalid("'*' is only allowed as a trailing ':*'"));
                    }
                    if body.is_some_and(|b| b.ends_with(' ')) {
                        return Err(invalid("no space allowed before ':*'"));
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum Pattern<'a> {
    Any,
    Exact(&'a str),
    Prefix(&'a str),
    WithSpec { tool: &'a str, spec: &'a str },
}

fn parse_pattern(p: &str) -> std::result::Result<Pattern<'_>, String> {
    let p = p.trim();
    if p.is_empty() {
        return Err("empty pattern".into());
    }
    if p == "*" {
        return Ok(Pattern::Any);
    }
    if let Some(open) = p.find('(') {
        if open == 0 || !p.ends_with(')') {
            return Err("expected Tool(spec)".into());
        }
        let spec = &p[open + 1..p.len() - 1];
        if spec.trim().is_empty() {
            return Err("empty spec".into());
        }
        return Ok(Pattern::WithSpec {
            tool: &p[..open],
            spec,
        });
    }
    if let Some(prefix) = p.strip_suffix('*') {
        if prefix.contains('*') {
            return Err("'*' is only allowed at the end".into());
        }
        return Ok(Pattern::Prefix(prefix));
    }
    if p.contains('*') {
        return Err("'*' is only allowed at the end".into());
    }
    Ok(Pattern::Exact(p))
}

// ---- command matching ----------------------------------------------------

/// Allow side: only plain, shell-inert commands may be matched. Returns the command with runs
/// of spaces collapsed, or `None` if it contains anything that could smuggle in a second command.
fn simple_command(command: &str) -> Option<String> {
    let command = command.trim();
    let simple = command
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || " -_./=:,@+%".contains(c));
    if !simple || command.split(' ').any(|t| t.starts_with("--output")) {
        return None;
    }
    Some(
        command
            .split(' ')
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    )
}

fn allow_command_matches(spec: &str, command: &str) -> bool {
    let Some(command) = simple_command(command) else {
        return false;
    };
    match spec.strip_suffix(":*") {
        Some(prefix) => command == prefix || command.starts_with(&format!("{prefix} ")),
        None => command == spec,
    }
}

/// Deny/ask side: fuzzy, fail-closed tokenization (case, path and `.exe` insensitive).
fn tokenize(s: &str) -> Vec<String> {
    s.to_lowercase()
        .replace(['\'', '"'], "")
        .split(|c: char| c.is_whitespace() || ";|&(){}[]<>$`,".contains(c))
        .filter(|t| !t.is_empty())
        .map(|t| {
            let base = t.rsplit(['/', '\\']).next().unwrap_or(t);
            base.strip_suffix(".exe").unwrap_or(base).to_string()
        })
        .filter(|t| !t.is_empty())
        .collect()
}

/// True if the spec tokens appear, in order but not necessarily contiguously, in the command.
fn restrictive_command_matches(spec: &str, command: &str) -> bool {
    let spec = spec.strip_suffix(":*").unwrap_or(spec);
    let spec_tokens = tokenize(spec);
    let command_tokens = tokenize(command);
    let mut it = command_tokens.iter();
    spec_tokens.iter().all(|s| it.any(|c| c == s))
}

// ---- path / url matching -------------------------------------------------

/// Lexically normalizes a path: `\` -> `/`, lowercase, trailing dots/spaces trimmed from every
/// component, `//` collapsed, `.` dropped, `..` resolved. Returns (is_absolute, normalized).
fn lexical_normalize(raw: &str) -> (bool, String) {
    let s = raw.replace('\\', "/").to_ascii_lowercase();
    let b = s.as_bytes();
    let (prefix, rest) = if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        (&s[..2], &s[2..])
    } else {
        ("", &s[..])
    };
    let abs = !prefix.is_empty() || rest.starts_with('/');
    let mut stack: Vec<&str> = Vec::new();
    for c in rest.split('/') {
        match c {
            "" | "." => {}
            ".." => {
                if stack.last().is_some_and(|l| *l != "..") {
                    stack.pop();
                } else if !abs {
                    stack.push("..");
                }
            }
            _ => {
                // NTFS alternate data streams / index aliases: `.git:$I30:...` is `.git`.
                let c = c.split(':').next().unwrap_or(c);
                let t = c.trim_end_matches(['.', ' ']);
                if !t.is_empty() {
                    stack.push(t);
                }
            }
        }
    }
    let joined = stack.join("/");
    (
        abs,
        if abs {
            format!("{prefix}/{joined}")
        } else {
            joined
        },
    )
}

/// Patterns: `\` -> `/`, lowercase, leading `./` stripped. Returns (is_absolute, pattern).
fn normalize_pattern(spec: &str) -> (bool, String) {
    let mut p = spec.trim().replace('\\', "/").to_ascii_lowercase();
    while let Some(rest) = p.strip_prefix("./") {
        p = rest.to_string();
    }
    let b = p.as_bytes();
    let abs = p.starts_with('/') || (b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':');
    (abs, p)
}

/// Allow-side guard on the raw input path: reject traversal and Windows path tricks.
fn raw_path_is_clean(raw: &str) -> bool {
    let b = raw.as_bytes();
    let has_drive = b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':';
    let rest = if has_drive { &raw[2..] } else { raw };
    !raw.contains('~')
        && !rest.contains(':')
        && raw
            .split(['/', '\\'])
            .all(|c| c != ".." && (c == "." || !(c.ends_with('.') || c.ends_with(' '))))
}

#[derive(Clone, Copy)]
enum Glob {
    Lit(char),
    Star,
    DoubleStar,
    Question,
}

fn parse_glob(pattern: &str) -> Vec<Glob> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if chars.get(i + 1) == Some(&'*') => {
                out.push(Glob::DoubleStar);
                i += 2;
            }
            '*' => {
                out.push(Glob::Star);
                i += 1;
            }
            '?' => {
                out.push(Glob::Question);
                i += 1;
            }
            c => {
                out.push(Glob::Lit(c));
                i += 1;
            }
        }
    }
    out
}

fn glob_match_tokens(pat: &[Glob], text: &[char]) -> bool {
    // dp[j] == pattern consumed so far matches text[..j]
    let mut dp = vec![false; text.len() + 1];
    dp[0] = true;
    for g in pat {
        let mut next = vec![false; text.len() + 1];
        match g {
            Glob::Lit(c) => {
                for j in 0..text.len() {
                    next[j + 1] = dp[j] && text[j] == *c;
                }
            }
            Glob::Question => {
                for j in 0..text.len() {
                    next[j + 1] = dp[j] && text[j] != '/';
                }
            }
            Glob::Star => {
                next[0] = dp[0];
                for j in 0..text.len() {
                    next[j + 1] = dp[j + 1] || (next[j] && text[j] != '/');
                }
            }
            Glob::DoubleStar => {
                next[0] = dp[0];
                for j in 0..text.len() {
                    next[j + 1] = dp[j + 1] || next[j];
                }
            }
        }
        dp = next;
    }
    dp[text.len()]
}

fn glob_matches(pattern: &str, path: &str) -> bool {
    let text: Vec<char> = path.chars().collect();
    if glob_match_tokens(&parse_glob(pattern), &text) {
        return true;
    }
    // A leading `**/` may also match zero directories.
    match pattern.strip_prefix("**/") {
        Some(rest) => glob_match_tokens(&parse_glob(rest), &text),
        None => false,
    }
}

fn input_path(input: &Value) -> Option<&str> {
    ["file_path", "path", "notebook_path"]
        .iter()
        .find_map(|k| input.get(k).and_then(Value::as_str))
}

fn path_spec_matches(spec: &str, input: &Value, action: Action, workspace: Option<&str>) -> bool {
    let Some(raw) = input_path(input) else {
        return action != Action::Allow;
    };
    if action == Action::Allow && !raw_path_is_clean(raw) {
        return false;
    }
    let (abs, full) = lexical_normalize(raw);
    // Candidate paths: the workspace-relative form (if any) first, then the full normalized path.
    let mut relative: Option<String> = None;
    if let (true, Some(ws)) = (abs, workspace) {
        let ws = ws.trim_end_matches('/');
        if full == ws || full == format!("{ws}/") {
            relative = Some(String::new());
        } else if let Some(rel) = full.strip_prefix(&format!("{ws}/")) {
            relative = Some(rel.to_string());
        }
    }
    let (pattern_abs, pattern) = normalize_pattern(spec);
    if action == Action::Allow {
        let (path_abs, path) = match relative {
            Some(rel) => (false, rel),
            None => (abs, full),
        };
        return (!path_abs || pattern_abs) && glob_matches(&pattern, &path);
    }
    // Deny/ask fail closed: test every candidate, whole and at every suffix after a '/'.
    relative.iter().chain(std::iter::once(&full)).any(|path| {
        glob_matches(&pattern, path)
            || path
                .match_indices('/')
                .any(|(i, _)| glob_matches(&pattern, &path[i + 1..]))
    })
}

struct UrlInfo {
    /// Hosts the URL could be read as (lowercased, port and trailing dot removed).
    hosts: Vec<String>,
    has_userinfo: bool,
    /// Authority contains `%` or `\`, which parsers disagree on.
    suspicious: bool,
}

fn parse_url(url: &str) -> Option<UrlInfo> {
    let rest = &url[url.find("://")? + 3..];
    let raw_auth = &rest[..rest.find(['/', '?', '#']).unwrap_or(rest.len())];
    let suspicious = raw_auth.contains(['%', '\\']);
    let authority = &raw_auth[..raw_auth.find('\\').unwrap_or(raw_auth.len())];
    let host_of = |s: &str| {
        let host = &s[..s.find(':').unwrap_or(s.len())];
        host.to_ascii_lowercase().trim_end_matches('.').to_string()
    };
    let mut hosts = vec![host_of(authority)];
    if let Some((_, after)) = authority.rsplit_once('@') {
        hosts.push(host_of(after));
    }
    Some(UrlInfo {
        hosts,
        has_userinfo: authority.contains('@'),
        suspicious,
    })
}

fn webfetch_domain_matches(domain: &str, input: &Value, action: Action) -> bool {
    let domain = domain.trim().to_lowercase();
    let host_matches = |h: &String| *h == domain || h.ends_with(&format!(".{domain}"));
    let plain_host = |h: &String| {
        !h.is_empty()
            && h.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
    };
    match input.get("url").and_then(Value::as_str).and_then(parse_url) {
        None => action != Action::Allow,
        Some(u) => {
            if action == Action::Allow {
                !u.has_userinfo
                    && !u.suspicious
                    && u.hosts.iter().all(|h| plain_host(h) && host_matches(h))
            } else {
                // Fail closed on `%`/`\` and on any real-host character outside [a-z0-9.-].
                u.suspicious
                    || u.hosts.last().is_some_and(|h| !plain_host(h))
                    || u.hosts.iter().any(host_matches)
            }
        }
    }
}

fn pattern_matches(
    pattern: &str,
    tool: &str,
    input: &Value,
    action: Action,
    workspace: Option<&str>,
) -> bool {
    match parse_pattern(pattern) {
        Err(_) => false,
        Ok(Pattern::Any) => true,
        Ok(Pattern::Exact(name)) => name == tool,
        Ok(Pattern::Prefix(prefix)) => tool.starts_with(prefix),
        Ok(Pattern::WithSpec { tool: name, spec }) => {
            if name != tool {
                return false;
            }
            if COMMAND_TOOLS.contains(&name) {
                let Some(command) = input.get("command").and_then(Value::as_str) else {
                    return action != Action::Allow;
                };
                if action == Action::Allow {
                    allow_command_matches(spec, command)
                } else {
                    restrictive_command_matches(spec, command)
                }
            } else if FILE_TOOLS.contains(&name) {
                path_spec_matches(spec, input, action, workspace)
            } else if let (true, Some(domain)) = (
                name == "WebFetch",
                spec.trim_start().strip_prefix("domain:"),
            ) {
                webfetch_domain_matches(domain, input, action)
            } else {
                // Unknown spec: the CLI enforces it; fail closed on deny/ask, never allow.
                action != Action::Allow
            }
        }
    }
}
