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

use crate::library::{check_name, LibraryKind};
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
    /// What an MCP tool gets when no rule and no read-only flag decides (`resolve_mcp`, step 6).
    /// A policy stored before it existed reads as `ask`.
    #[serde(default = "default_mcp_action")]
    pub mcp_default: Action,
}

fn default_mcp_action() -> Action {
    Action::Ask
}

/// What the Library knows about the server and the tool of an MCP call, for `resolve_mcp`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpTrust {
    /// The server is held (its status is not `accepted`) or the tool is not in its accepted list.
    pub held: bool,
    /// The accepted list gives the tool `annotations.readOnlyHint: true`.
    pub read_only: bool,
}

/// Dots' own check-in tools by policy name (check-ins spec section 4): every preset allows
/// them, and the CLI never sees their rules.
pub const CHECK_IN_TOOLS: [&str; 3] = ["Notify", "Ask", "FollowUp"];

/// Dots' own memory tools (`remember`, `forget`, `recall`) by their one policy name (dot memory
/// spec section 3): treated as the check-in tools are. A rule names it alone: `Memory(...)` is
/// refused.
pub const MEMORY_TOOL: &str = crate::memory::POLICY_NAME;

/// Dots' own app tools by their policy names (app library spec section 6): treated as the
/// check-in tools are.
pub const APP_TOOLS: [&str; 2] = [crate::apps::APP_POLICY, crate::apps::PROPOSE_POLICY];

/// The refusal of a rule `ProposeApp(...)`.
pub const PROPOSE_RULE: &str = "a ProposeApp rule names no argument";

/// Dots' own workflow tools (`start_workflow`) by their one policy name (teams spec section 7):
/// treated as `Memory` is. A rule names it alone: `Workflow(...)` is refused.
pub const WORKFLOW_TOOL: &str = crate::team_tools::POLICY_NAME;

/// The refusal of a rule `Workflow(...)`.
pub const WORKFLOW_RULE: &str =
    "a Workflow rule takes no argument: the rule Workflow covers every workflow tool";

/// Dots' own `propose_instructions` by its policy name (conversational chat spec section 4):
/// treated as `Memory` is. A rule names it alone: `Instructions(...)` is refused.
pub const INSTRUCTIONS_TOOL: &str = crate::instructions::POLICY_NAME;

/// The refusal of a rule `Instructions(...)`.
pub const INSTRUCTIONS_RULE: &str = "an Instructions rule names no argument";

/// A tool whose ask rule wins over an allow rule: the check-in tools, `Memory`, `Workflow`,
/// the app tools and `Instructions`.
fn ask_wins(tool: &str) -> bool {
    CHECK_IN_TOOLS.contains(&tool)
        || tool == MEMORY_TOOL
        || tool == WORKFLOW_TOOL
        || APP_TOOLS.contains(&tool)
        || tool == INSTRUCTIONS_TOOL
}

/// The refusal of a rule `Memory(...)`.
pub const MEMORY_RULE: &str =
    "a Memory rule takes no argument: the rule Memory covers remember, forget and recall";

const READ_TOOLS: &[&str] = &["Read", "LS", "Glob", "Grep"];
const COMMAND_TOOLS: &[&str] = &["Bash", "PowerShell"];
/// Commands that run whatever follows them: a prefix rule through one would allow any command.
const WRAPPER_COMMANDS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "dash",
    "fish",
    "cmd",
    "pwsh",
    "powershell",
    "env",
    "sudo",
    "doas",
    "xargs",
    "nohup",
    "time",
    "nice",
    "timeout",
    "start",
    "call",
    "wsl",
    "exec",
    "command",
    "eval",
    "su",
    "runas",
    "stdbuf",
    "chroot",
    "watch",
    // PowerShell launchers and their aliases: the local Bash tool runs PowerShell.
    "start-process",
    "saps",
    "invoke-expression",
    "iex",
    "invoke-command",
    "icm",
    "start-job",
    "sajb",
    "invoke-item",
    "ii",
];
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
        let mut mcp_default = Action::Ask;
        let default = match preset {
            Preset::ReadOnly => {
                rules.extend(READ_TOOLS.iter().map(|t| Rule::new(t, Action::Allow)));
                for t in ["Write", "Edit", "NotebookEdit", "Bash", "PowerShell"] {
                    rules.push(Rule::new(t, Action::Deny));
                }
                mcp_default = Action::Deny;
                Action::Deny
            }
            // `Custom` starts from the Sandboxed rules for the user to edit.
            Preset::Sandboxed | Preset::Custom => {
                rules.extend(READ_TOOLS.iter().map(|t| Rule::new(t, Action::Allow)));
                rules.push(Rule::new("Write(./**)", Action::Allow));
                rules.push(Rule::new("Edit(./**)", Action::Allow));
                // No `git commit`: commit hooks run arbitrary code, so commits ask.
                for git in ["status", "diff", "add"] {
                    rules.push(Rule::new(&format!("Bash(git {git}:*)"), Action::Allow));
                }
                // `.git` holds hooks and config; `.claude` holds settings and hooks the CLI
                // would load on a later run. Both execute code, so neither is writable.
                for dir in [".git", ".claude"] {
                    for tool in ["Write", "Edit"] {
                        rules.push(Rule::new(&format!("{tool}(**/{dir}/**)"), Action::Deny));
                        rules.push(Rule::new(&format!("{tool}(**/{dir})"), Action::Deny));
                    }
                }
                Action::Ask
            }
            Preset::Trusted => {
                rules.push(Rule::new("Bash(git push:*)", Action::Ask));
                rules.push(Rule::new("PowerShell(git push:*)", Action::Ask));
                Action::Allow
            }
        };
        // Every preset lets the model read the skills and the context collections the dot uses,
        // check in with the person (notes, a question, follow-ups within their limits) and
        // keep its memory.
        rules.push(Rule::new("Skill", Action::Allow));
        rules.push(Rule::new("Context", Action::Allow));
        rules.extend(CHECK_IN_TOOLS.map(|tool| Rule::new(tool, Action::Allow)));
        rules.push(Rule::new(MEMORY_TOOL, Action::Allow));
        // And start its team's workflows from a chat, which Sandboxed and Custom ask before
        // (teams spec section 7, as decided by the person).
        let workflow = match preset {
            Preset::Sandboxed | Preset::Custom => Action::Ask,
            Preset::ReadOnly | Preset::Trusted => Action::Allow,
        };
        rules.push(Rule::new(WORKFLOW_TOOL, workflow));
        // And use its apps and propose new ones: the review card is a proposal's gate.
        rules.extend(APP_TOOLS.map(|tool| Rule::new(tool, Action::Allow)));
        // And propose new instructions for itself from a chat: the card is the proposal's gate
        // (conversational chat spec section 4).
        rules.push(Rule::new(INSTRUCTIONS_TOOL, Action::Allow));
        Policy {
            preset,
            rules,
            default,
            mcp_default,
        }
    }

    /// Deny rules win over allow rules, which win over ask rules; otherwise `default`.
    pub fn resolve(&self, tool: &str, input: &Value) -> Action {
        self.resolve_in(tool, input, None)
    }

    /// Like `resolve`, but absolute file paths inside `workspace` are treated as relative to it.
    pub fn resolve_in(&self, tool: &str, input: &Value, workspace: Option<&Path>) -> Action {
        let ws = workspace_key(workspace);
        // A check-in tool or `Memory` set to ask means it, even under the preset's allow
        // (check-ins spec section 4, dot memory spec section 3); every other tool keeps
        // deny > allow > ask, where a narrow allow beats a broad ask.
        let order = if ask_wins(tool.trim()) {
            [Action::Deny, Action::Ask, Action::Allow]
        } else {
            [Action::Deny, Action::Allow, Action::Ask]
        };
        for action in order {
            if self.rule_matches(action, tool, input, ws.as_deref()) {
                return action;
            }
        }
        self.default
    }

    /// The check of an MCP tool call (`tool` is `mcp__<server>__<tool>`). The first match wins:
    /// a deny rule; a held server or an unlisted tool (ask); an allow rule; an ask rule; the
    /// accepted `readOnlyHint` (allow); `mcp_default`. MCP tools never fall to `default`.
    pub fn resolve_mcp(&self, tool: &str, input: &Value, trust: McpTrust) -> Action {
        if self.rule_matches(Action::Deny, tool, input, None) {
            return Action::Deny;
        }
        if trust.held {
            return Action::Ask;
        }
        if self.rule_matches(Action::Allow, tool, input, None) {
            return Action::Allow;
        }
        if self.rule_matches(Action::Ask, tool, input, None) {
            return Action::Ask;
        }
        if trust.read_only {
            return Action::Allow;
        }
        self.mcp_default
    }

    /// True when a deny rule matches the call, so `resolve_in` returns `deny` whatever the
    /// other rules and the default say. False for a call that only the default denies. The
    /// walking tools use it to tell a directory the user closed from one no rule mentions.
    pub fn denied_by_rule(&self, tool: &str, input: &Value, workspace: Option<&Path>) -> bool {
        let ws = workspace_key(workspace);
        self.rule_matches(Action::Deny, tool, input, ws.as_deref())
    }

    /// True if a rule with this `action` matches the call. `ws` comes from `workspace_key`.
    fn rule_matches(&self, action: Action, tool: &str, input: &Value, ws: Option<&str>) -> bool {
        self.rules
            .iter()
            .any(|r| r.action == action && pattern_matches(&r.tool, tool, input, action, ws))
    }

    /// (allow patterns, deny patterns) for `--allowedTools` / `--disallowedTools`. A rule of
    /// `Context`, of a check-in tool (`CHECK_IN_TOOLS`) or of the browser (`Browser`,
    /// `BrowserRisky`) is left out: the CLI has no such tool, and Dots checks each call of its
    /// tools against these rules itself (context library spec section 4, check-ins spec
    /// section 4, browser spec section 6).
    pub fn cli_lists(&self) -> (Vec<String>, Vec<String>) {
        let dots_only = |tool: &str| {
            let tool = tool.trim();
            [
                "Context",
                "Browser",
                "BrowserRisky",
                MEMORY_TOOL,
                WORKFLOW_TOOL,
                INSTRUCTIONS_TOOL,
            ]
            .iter()
            .chain(CHECK_IN_TOOLS.iter())
            .chain(APP_TOOLS.iter())
            .any(|name| {
                tool == *name
                    || tool
                        .strip_prefix(name)
                        .is_some_and(|rest| rest.starts_with('('))
            })
        };
        let pick = |a: Action| -> Vec<String> {
            self.rules
                .iter()
                .filter(|r| r.action == a && !dots_only(&r.tool))
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
            // The browser's rules name one site each (browser spec section 6).
            if matches!(pattern, Pattern::Exact("Browser" | "BrowserRisky")) {
                return Err(invalid(crate::browser::sites::BARE_RULE));
            }
            if let Pattern::WithSpec { tool, spec } = pattern {
                if tool == MEMORY_TOOL {
                    return Err(invalid(MEMORY_RULE));
                }
                if tool == crate::apps::PROPOSE_POLICY {
                    return Err(invalid(PROPOSE_RULE));
                }
                if tool == crate::apps::APP_POLICY {
                    crate::apps::check_rule_spec(spec).map_err(|e| invalid(&e))?;
                }
                if tool == WORKFLOW_TOOL {
                    return Err(invalid(WORKFLOW_RULE));
                }
                if tool == INSTRUCTIONS_TOOL {
                    return Err(invalid(INSTRUCTIONS_RULE));
                }
                if matches!(tool, "Browser" | "BrowserRisky") {
                    crate::browser::sites::check_site(spec).map_err(|e| invalid(&e))?;
                }
                if tool == "Context" && check_name(LibraryKind::Context, spec.trim()).is_err() {
                    return Err(invalid(
                        "a Context rule names one collection: lower-case letters, digits, '-' and '_'",
                    ));
                }
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

/// One allow rule that Dots offers to save for a call that asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleCandidate {
    /// The rule's pattern, as `Rule::tool` holds it.
    pub pattern: String,
    /// What the pattern covers, for the person who picks it.
    pub label: String,
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

/// The normalized workspace path that file rules are matched against; `None` unless absolute.
fn workspace_key(workspace: Option<&Path>) -> Option<String> {
    let (abs, normalized) = lexical_normalize(&workspace?.to_string_lossy());
    abs.then_some(normalized)
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
    // Deny/ask fail closed: test every candidate, whole and at every suffix after a '/'. A
    // relative input (the local engine's form) is also placed inside the workspace, so absolute
    // patterns match it. That joined path is normalized again so that a leading `..` which
    // leaves and re-enters the workspace cannot dodge an absolute pattern. It is matched whole
    // and only against absolute patterns: its suffixes would let a relative rule match the
    // workspace's own location (`users/**` against `C:/Users/.../worktrees/RUN`). `dir/**` also
    // covers `dir` itself.
    let mut candidates: Vec<String> = relative.into_iter().collect();
    let mut joined: Option<String> = None;
    if let (false, Some(ws)) = (abs, workspace) {
        let ws = ws.trim_end_matches('/');
        joined = Some(lexical_normalize(&format!("{ws}/{full}")).1);
    }
    candidates.push(full);
    let dir = pattern.strip_suffix("/**").filter(|d| !d.is_empty());
    let hit = |p: &str| glob_matches(&pattern, p) || dir.is_some_and(|d| glob_matches(d, p));
    (pattern_abs && joined.as_deref().is_some_and(hit))
        || candidates
            .iter()
            .any(|path| hit(path) || path.match_indices('/').any(|(i, _)| hit(&path[i + 1..])))
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

/// A clean input path with `\` as `/` and its empty and `.` components dropped; its case is
/// kept, because the pattern made from it is shown to a person. Returns (is_absolute, path).
/// For a path that passes `raw_path_is_clean`, the lowercase of the result is what
/// `lexical_normalize` returns.
fn tidy_path(raw: &str) -> (bool, String) {
    let s = raw.replace('\\', "/");
    let b = s.as_bytes();
    let (prefix, rest) = if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        (&s[..2], &s[2..])
    } else {
        ("", &s[..])
    };
    let abs = !prefix.is_empty() || rest.starts_with('/');
    let joined = rest
        .split('/')
        .filter(|c| !c.is_empty() && *c != ".")
        .collect::<Vec<_>>()
        .join("/");
    (
        abs,
        if abs {
            format!("{prefix}/{joined}")
        } else {
            joined
        },
    )
}

/// The allow rules Dots offers to save when a person approves this call, narrowest first.
/// Empty when the call can only be approved one at a time.
///
/// - A command tool: the exact command, the command without its last token as a prefix, and
///   its first two tokens as a prefix. Only for a command `simple_command` accepts. A command
///   that starts with a shell or wrapper (`WRAPPER_COMMANDS`, also given with its path) or sets a
///   variable gets only the exact rule. A two-token prefix is left out when its second token
///   starts with `-` or `/`.
/// - A file tool: the path, and everything under its directory. A path inside `workspace` is
///   written relative to it. Not for the workspace root or a drive root as the directory, and
///   not at all for a UNC, device or drive-relative path.
/// - `WebFetch`: the host.
/// - An MCP tool (`mcp__<server>__<tool>`): its name, and `mcp__<server>__*` for the server.
/// - Any other tool: its name.
///
/// A bare command or file tool (`Bash`, `Write`) and a one-token command prefix (`Bash(git:*)`)
/// are never offered. Every candidate returned is a valid rule that, as an allow rule, matches
/// this call: one that is not is left out, so the caller can save any of them unchecked.
pub fn allow_candidates(tool: &str, input: &Value, workspace: Option<&Path>) -> Vec<RuleCandidate> {
    // A browser call offers its site's rules and nothing else (browser spec section 3.6).
    if let Some(candidates) = crate::browser::gate::rule_candidates(tool, input) {
        return candidates;
    }
    let ws = workspace_key(workspace);
    let mut found: Vec<(String, String)> = Vec::new();
    if COMMAND_TOOLS.contains(&tool) {
        let command = input
            .get("command")
            .and_then(Value::as_str)
            .and_then(simple_command);
        if let Some(command) = command {
            let tokens: Vec<&str> = command.split(' ').collect();
            found.push((format!("{tool}({command})"), "this exact command".into()));
            // The program's name without its directory and `.exe`, as the deny side reads it.
            let first = tokens[0]
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            let first = first.strip_suffix(".exe").unwrap_or(&first);
            let wrapper = WRAPPER_COMMANDS.contains(&first) || tokens[0].contains('=');
            // An option or a path switch after the command name decides what the command does.
            let optional_second = tokens.get(1).is_some_and(|t| t.starts_with(['-', '/']));
            for len in [tokens.len().saturating_sub(1), 2] {
                if len >= 2 && len < tokens.len() && !wrapper && !(len == 2 && optional_second) {
                    let prefix = tokens[..len].join(" ");
                    found.push((
                        format!("{tool}({prefix}:*)"),
                        format!("commands that start with {prefix}"),
                    ));
                }
            }
        }
    } else if FILE_TOOLS.contains(&tool) {
        // UNC, device and drive-relative paths name places the rules below would misdescribe.
        let named_plainly = |r: &str| {
            let r = r.replace('\\', "/");
            let b = r.as_bytes();
            let unc = r.starts_with("//");
            let drive_relative = b.len() >= 2
                && b[0].is_ascii_alphabetic()
                && b[1] == b':'
                && b.get(2) != Some(&b'/');
            !unc && !drive_relative
        };
        let raw = input_path(input)
            .filter(|r| raw_path_is_clean(r) && !r.contains(['*', '?']) && named_plainly(r));
        if let Some(raw) = raw {
            let (abs, tidy) = tidy_path(raw);
            let inside = ws.as_deref().filter(|_| abs).and_then(|ws| {
                let ws = ws.trim_end_matches('/');
                tidy.to_ascii_lowercase()
                    .starts_with(&format!("{ws}/"))
                    .then(|| tidy[ws.len() + 1..].to_string())
            });
            let path = inside.unwrap_or(tidy);
            if !path.is_empty() {
                found.push((format!("{tool}({path})"), "this path".into()));
                if let Some((parent, _)) = path.rsplit_once('/') {
                    let is_root = parent.is_empty() || (parent.len() == 2 && parent.ends_with(':'));
                    if !is_root {
                        found.push((
                            format!("{tool}({parent}/**)"),
                            format!("everything under {parent}/"),
                        ));
                    }
                }
            }
        }
    } else if tool == "WebFetch" {
        let host = input
            .get("url")
            .and_then(Value::as_str)
            .and_then(parse_url)
            .filter(|u| !u.has_userinfo && !u.suspicious)
            .and_then(|u| u.hosts.into_iter().next());
        if let Some(host) = host {
            found.push((
                format!("WebFetch(domain:{host})"),
                "this host and its subdomains".into(),
            ));
        }
    } else if !tool.contains('*') {
        // A name that ends in `*` would be read as a prefix of other tools' names.
        found.push((tool.to_string(), "every call of this tool".into()));
        // An MCP tool also offers its whole server. The server name ends at the first `__`.
        let server = tool
            .strip_prefix("mcp__")
            .and_then(|rest| rest.split_once("__"))
            .filter(|(server, name)| !server.is_empty() && !name.is_empty());
        if let Some((server, _)) = server {
            found.push((
                format!("mcp__{server}__*"),
                format!("every tool of the server {server}"),
            ));
        }
    }
    let mut out: Vec<RuleCandidate> = Vec::new();
    for (pattern, label) in found {
        let rule = Policy {
            preset: Preset::Custom,
            rules: vec![Rule::new(&pattern, Action::Allow)],
            default: Action::Ask,
            mcp_default: Action::Ask,
        };
        if rule.validate().is_ok()
            && pattern_matches(&pattern, tool, input, Action::Allow, ws.as_deref())
            && !out.iter().any(|c| c.pattern == pattern)
        {
            out.push(RuleCandidate { pattern, label });
        }
    }
    out
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
            } else if name == crate::apps::APP_POLICY {
                // The argument names an app, a tool or the writing tools. The CLI never sees
                // `App` rules: a call without an `app` matches deny and ask rules only.
                crate::apps::rule_spec_matches(spec, input).unwrap_or(action != Action::Allow)
            } else if name == "Context" {
                // The argument names a collection. The CLI never sees `Context` rules, so this
                // is the only check: a call without a `collection` matches deny and ask rules.
                match input.get("collection").and_then(Value::as_str) {
                    Some(collection) => collection.trim() == spec.trim(),
                    None => action != Action::Allow,
                }
            } else {
                // Unknown spec: the CLI enforces it; fail closed on deny/ask, never allow.
                action != Action::Allow
            }
        }
    }
}
