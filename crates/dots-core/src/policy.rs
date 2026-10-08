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
        Self { tool: tool.to_string(), action }
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
            Preset::Sandboxed | Preset::Custom => {
                rules.extend(READ_TOOLS.iter().map(|t| Rule::new(t, Action::Allow)));
                rules.push(Rule::new("Write", Action::Allow));
                rules.push(Rule::new("Edit", Action::Allow));
                for git in ["status", "diff", "log", "add", "commit"] {
                    rules.push(Rule::new(&format!("Bash(git {git}:*)"), Action::Allow));
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
        Policy { preset, rules, default }
    }

    /// Deny rules win over allow rules, which win over ask rules; otherwise `default`.
    pub fn resolve(&self, tool: &str, input: &Value) -> Action {
        for action in [Action::Deny, Action::Allow, Action::Ask] {
            if self
                .rules
                .iter()
                .any(|r| r.action == action && pattern_matches(&r.tool, tool, input, action))
            {
                return action;
            }
        }
        self.default
    }

    /// (allow patterns, deny patterns) for `--allowedTools` / `--disallowedTools`.
    pub fn cli_lists(&self) -> (Vec<String>, Vec<String>) {
        let pick = |a: Action| -> Vec<String> {
            self.rules.iter().filter(|r| r.action == a).map(|r| r.tool.clone()).collect()
        };
        (pick(Action::Allow), pick(Action::Deny))
    }

    pub fn validate(&self) -> Result<()> {
        for r in &self.rules {
            parse_pattern(&r.tool)
                .map_err(|e| Error::Invalid(format!("policy rule {:?}: {e}", r.tool)))?;
        }
        Ok(())
    }
}

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
        return Ok(Pattern::WithSpec { tool: &p[..open], spec });
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

fn is_compound(cmd: &str) -> bool {
    cmd.contains("$(") || cmd.chars().any(|c| matches!(c, ';' | '|' | '&' | '\n' | '`' | '>' | '<'))
}

fn segments(cmd: &str) -> impl Iterator<Item = &str> {
    cmd.split(|c| matches!(c, ';' | '|' | '&' | '\n'))
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn command_matches(spec: &str, command: &str) -> bool {
    match spec.strip_suffix(":*") {
        Some(prefix) => command == prefix || command.starts_with(&format!("{prefix} ")),
        None => command == spec,
    }
}

fn pattern_matches(pattern: &str, tool: &str, input: &Value, action: Action) -> bool {
    match parse_pattern(pattern) {
        Err(_) => false,
        Ok(Pattern::Any) => true,
        Ok(Pattern::Exact(name)) => name == tool,
        Ok(Pattern::Prefix(prefix)) => tool.starts_with(prefix),
        Ok(Pattern::WithSpec { tool: name, spec }) => {
            if name != tool || !COMMAND_TOOLS.contains(&name) {
                return false;
            }
            let Some(command) = input.get("command").and_then(Value::as_str) else {
                return false;
            };
            let command = command.trim();
            if action == Action::Allow {
                !is_compound(command) && command_matches(spec, command)
            } else {
                segments(command).any(|seg| command_matches(spec, seg))
            }
        }
    }
}
