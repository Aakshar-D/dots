//! A Claude Code agent file (`.claude/agents/<name>.md`): a frontmatter of `name`,
//! `description`, `tools` and `model`, and a body that is the agent's system prompt. Read here
//! for importing it as a dot (import agents spec section 3).

use std::fs;
use std::path::{Path, PathBuf};

use super::frontmatter::parse_frontmatter;
use crate::policy::{Action, Policy, Preset, Rule};

/// An agent file larger than this is not read.
pub const MAX_AGENT_BYTES: u64 = 256 * 1024;

/// How deep `agent_files` looks under its folder.
const MAX_DEPTH: usize = 6;

/// Tools an import allows by their bare name.
const SAFE_TOOLS: &[&str] = &[
    "Read",
    "LS",
    "Glob",
    "Grep",
    "WebSearch",
    "TodoWrite",
    "Task",
    "Agent",
    "NotebookRead",
    "Skill",
    "BashOutput",
    "KillShell",
    "ExitPlanMode",
];

/// Tools an import never allows bare: a rule of the name alone would let them run anything or
/// write anywhere, so they ask.
const RISKY_TOOLS: &[&str] = &[
    "Bash",
    "PowerShell",
    "Write",
    "Edit",
    "MultiEdit",
    "NotebookEdit",
    "WebFetch",
];

/// The `tools` an agent file gives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentTools {
    /// No `tools` key: the agent has every tool.
    All,
    /// The tools it lists (maybe none).
    Listed(Vec<String>),
    /// A `tools` value Dots cannot read: it must not be taken as "every tool".
    Unreadable,
}

impl AgentTools {
    /// The tools as the import dialog shows them: `None` for every tool, nothing listed for an
    /// unreadable value.
    pub fn shown(&self) -> Option<Vec<String>> {
        match self {
            AgentTools::All => None,
            AgentTools::Listed(tools) => Some(tools.clone()),
            AgentTools::Unreadable => Some(Vec::new()),
        }
    }
}

/// What an agent file says, before it becomes a dot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFile {
    /// The frontmatter `name`, else the file name without `.md`, as written.
    pub name: String,
    pub description: Option<String>,
    pub tools: AgentTools,
    pub model: Option<String>,
    /// The body, trimmed; never empty.
    pub prompt: String,
}

/// Reads the agent file at `path`. The error is a reason for the person: too large, unreadable,
/// no frontmatter Dots can read, no prompt.
pub fn read_agent(path: &Path) -> Result<AgentFile, String> {
    let size = fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_AGENT_BYTES {
        return Err("it is larger than 256 KiB".into());
    }
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let fm = parse_frontmatter(&text);
    if !fm.parsed {
        return Err("it has no frontmatter Dots can read".into());
    }
    let prompt = fm.body.trim().to_string();
    if prompt.is_empty() {
        return Err("it has no instructions".into());
    }
    let field = |key: &str| {
        fm.fields
            .get(key)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(AgentFile {
        name: field("name").unwrap_or(stem),
        description: field("description"),
        tools: if fm.unreadable.contains("tools") {
            AgentTools::Unreadable
        } else {
            fm.list("tools").map_or(AgentTools::All, AgentTools::Listed)
        },
        model: field("model"),
        prompt,
    })
}

/// The `.md` files under `dir` and its folders, sorted. A missing folder gives none.
pub fn agent_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                if depth < MAX_DEPTH {
                    walk(&path, depth + 1, out);
                }
            } else if kind.is_file()
                && path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("md"))
            {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, 0, &mut out);
    out.sort();
    out
}

/// `raw` as a dot name: 1-64 of `A-Za-z0-9-_`. Other characters become `-`, runs of `-`
/// collapse and none is left at either end; nothing left gives `agent`.
pub fn dot_name(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.trim().chars() {
        let c = if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            c
        } else {
            '-'
        };
        if c == '-' && (out.is_empty() || out.ends_with('-')) {
            continue;
        }
        out.push(c);
    }
    out.truncate(64);
    let out = out.trim_end_matches('-');
    if out.is_empty() {
        return "agent".into();
    }
    // A name Windows keeps for a device, with or without an extension, would make a folder
    // (the agent's workdir) that cannot be used.
    let stem = raw
        .trim()
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    if is_reserved(&stem) || is_reserved(&out.to_ascii_lowercase()) {
        let keep = out.len().min(64 - "-agent".len());
        return format!("{}-agent", out[..keep].trim_end_matches('-'));
    }
    out.to_string()
}

/// A Windows device name: `con`, `prn`, `aux`, `nul`, `com1`-`com9`, `lpt1`-`lpt9`.
fn is_reserved(name: &str) -> bool {
    if matches!(name, "con" | "prn" | "aux" | "nul") {
        return true;
    }
    let digit = |rest: &str| rest.len() == 1 && rest.chars().all(|c| ('1'..='9').contains(&c));
    name.strip_prefix("com").is_some_and(digit) || name.strip_prefix("lpt").is_some_and(digit)
}

/// The dot's model for an agent's `model`: Dots' own `sonnet`, `opus` or `haiku`; a full id as
/// it is; `None` (the dot default) for `inherit` or nothing.
pub fn agent_model(raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    let lower = raw.to_ascii_lowercase();
    match lower.as_str() {
        "" | "inherit" => None,
        "sonnet" | "opus" | "haiku" => Some(lower),
        _ => Some(raw.to_string()),
    }
}

/// Whether a Sandboxed rule is one of its allows of a file or command tool, which an agent's
/// own tool list replaces.
fn preset_tool_allow(rule: &Rule) -> bool {
    rule.action == Action::Allow
        && (["Read", "LS", "Glob", "Grep", "Write(./**)", "Edit(./**)"]
            .contains(&rule.tool.as_str())
            || rule.tool.starts_with("Bash(git "))
}

/// The dot's policy for an agent's tools, and what the person should know about it. No list
/// gives the Sandboxed preset. A list gives the Custom preset (default ask) without its own
/// allows of file and command tools, with an allow rule for each known safe tool and an ask
/// rule for each risky one; unknown names and MCP tools are left out and named. A value Dots
/// cannot read gives that policy with nothing listed: only Dots' own tools run without asking.
pub fn agent_policy(tools: &AgentTools) -> (Policy, Vec<String>) {
    let mut policy = Policy::preset(Preset::Custom);
    policy.default = Action::Ask;
    policy.rules.retain(|r| !preset_tool_allow(r));
    let tools = match tools {
        AgentTools::All => return (Policy::preset(Preset::Sandboxed), Vec::new()),
        AgentTools::Unreadable => {
            let warning = "Its tools could not be read, so every tool asks before each use.";
            return (policy, vec![warning.into()]);
        }
        AgentTools::Listed(tools) => tools,
    };
    let mut warnings = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for tool in tools {
        let tool = tool.trim();
        if tool.is_empty() || seen.contains(&tool) {
            continue;
        }
        seen.push(tool);
        if SAFE_TOOLS.contains(&tool) {
            policy.rules.push(Rule::new(tool, Action::Allow));
        } else if RISKY_TOOLS.contains(&tool) {
            policy.rules.push(Rule::new(tool, Action::Ask));
            warnings.push(format!("{tool} asks before each use."));
        } else if tool.starts_with("mcp__") {
            warnings.push(format!(
                "{tool} is left out: the dot does not have that MCP server."
            ));
        } else {
            warnings.push(format!("{tool} is left out: Dots does not know this tool."));
        }
    }
    (policy, warnings)
}
