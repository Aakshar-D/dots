//! A company's Claude Code managed settings, and what they mean for a dot's policy.
//!
//! Claude Code reads `managed-settings.json` (and the drop-ins in `managed-settings.d/`) from a
//! system folder, and nothing in a user, project or `--settings` file overrides it. Dots only
//! reads that file to tell the person; it never changes what Dots passes to the CLI.
//!
//! Locations and keys checked against the Claude Code docs (code.claude.com, "Deploy managed
//! settings"): macOS `/Library/Application Support/ClaudeCode`, Linux and WSL
//! `/etc/claude-code`, Windows `C:\Program Files\ClaudeCode`. The docs say the legacy
//! `C:\ProgramData\ClaudeCode\managed-settings.json` is no longer read, so Dots does not look
//! there. Keys: `permissions.disableBypassPermissionsMode` (`"disable"`), `permissions.ask`,
//! `permissions.deny`, `allowManagedPermissionRulesOnly`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::EngineKind;
use crate::policy::{Action, Policy};

/// What the company's managed settings say that matters to Dots.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedPolicy {
    /// The `managed-settings.json` found (or, with only drop-ins, the first drop-in); `None`
    /// when the machine has no managed settings.
    pub path: Option<PathBuf>,
    /// `permissions.disableBypassPermissionsMode` is `"disable"`: the CLI cannot skip all asks.
    pub blocks_skipping_asks: bool,
    /// `permissions.ask` rules, which the CLI asks about whatever an allow rule says.
    pub forced_ask: Vec<String>,
    /// `permissions.deny` rules, which the CLI refuses.
    pub forced_deny: Vec<String>,
    /// `allowManagedPermissionRulesOnly`: allow rules from anywhere but the company are ignored.
    pub managed_rules_only: bool,
    /// Set when a managed file exists but could not be read; names the file.
    pub error: Option<String>,
}

/// The system folder Claude Code reads managed settings from on this platform.
pub fn system_dir() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\Program Files\ClaudeCode")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/ClaudeCode")
    } else {
        PathBuf::from("/etc/claude-code")
    }
}

/// The managed settings of this machine.
pub fn read() -> ManagedPolicy {
    read_from(&system_dir())
}

/// The managed settings under `dir`: `managed-settings.json`, then `managed-settings.d/*.json`
/// in name order. A missing file is no policy; a file that cannot be read or parsed sets
/// `error` (with its path) and is skipped.
pub fn read_from(dir: &Path) -> ManagedPolicy {
    let mut files = vec![dir.join("managed-settings.json")];
    if let Ok(entries) = std::fs::read_dir(dir.join("managed-settings.d")) {
        let mut drop_ins: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .collect();
        drop_ins.sort();
        files.extend(drop_ins);
    }
    let mut out = ManagedPolicy::default();
    for file in files {
        let text = match std::fs::read_to_string(&file) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                note_error(&mut out, &file, &e.to_string());
                continue;
            }
        };
        match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
            Ok(v @ Value::Object(_)) => {
                out.path.get_or_insert_with(|| file.clone());
                merge(&mut out, &v);
            }
            Ok(_) => note_error(&mut out, &file, "not a JSON object"),
            Err(e) => note_error(&mut out, &file, &e.to_string()),
        }
    }
    out
}

fn note_error(out: &mut ManagedPolicy, file: &Path, why: &str) {
    out.path.get_or_insert_with(|| file.to_path_buf());
    out.error.get_or_insert_with(|| {
        format!(
            "Couldn't read your company's Claude Code policy at {}: {why}",
            file.display()
        )
    });
}

fn merge(out: &mut ManagedPolicy, v: &Value) {
    let perms = v.get("permissions");
    if perms
        .and_then(|p| p.get("disableBypassPermissionsMode"))
        .and_then(Value::as_str)
        == Some("disable")
    {
        out.blocks_skipping_asks = true;
    }
    for (key, list) in [("ask", &mut out.forced_ask), ("deny", &mut out.forced_deny)] {
        let rules = perms.and_then(|p| p.get(key)).and_then(Value::as_array);
        for rule in rules.into_iter().flatten().filter_map(Value::as_str) {
            let rule = rule.trim().to_string();
            if !rule.is_empty() && !list.contains(&rule) {
                list.push(rule);
            }
        }
    }
    if v.get("allowManagedPermissionRulesOnly") == Some(&Value::Bool(true)) {
        out.managed_rules_only = true;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    /// The company asks about it, so the dot cannot do it without asking.
    Ask,
    /// The company refuses it.
    Deny,
    /// The company's rules are the only ones the CLI reads: the dot's own allow rules do not
    /// count.
    ManagedRulesOnly,
}

/// One thing the dot's policy would do without asking that the company policy does not allow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Conflict {
    pub kind: ConflictKind,
    /// The company's rule (`Bash(rm:*)`); empty for `managed_rules_only`.
    pub rule: String,
    /// The dot's allowing rule, or `"everything else"` when its default allows.
    pub dot_rule: String,
}

/// What the app tells the UI: the managed settings, and where one dot's policy runs into them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagedCheck {
    pub managed: ManagedPolicy,
    pub conflicts: Vec<Conflict>,
}

/// Reads the managed settings under `dir` and checks `policy` against them.
pub fn check(dir: &Path, engine: EngineKind, policy: &Policy) -> ManagedCheck {
    let managed = read_from(dir);
    let conflicts = conflicts(engine, policy, &managed);
    ManagedCheck { managed, conflicts }
}

/// Where `policy` would skip an ask or a refusal that `managed` keeps. Only a dot run by the
/// Claude Code CLI is affected. Bypass mode is not a conflict: Dots never passes it, so
/// `blocks_skipping_asks` changes nothing for a dot.
pub fn conflicts(engine: EngineKind, policy: &Policy, managed: &ManagedPolicy) -> Vec<Conflict> {
    let mut out: Vec<Conflict> = Vec::new();
    if engine != EngineKind::Claude {
        return out;
    }
    let (allow, _) = policy.cli_lists();
    let default_allows = policy.default == Action::Allow;
    if managed.managed_rules_only && (default_allows || !allow.is_empty()) {
        out.push(Conflict {
            kind: ConflictKind::ManagedRulesOnly,
            rule: String::new(),
            dot_rule: if allow.is_empty() {
                "everything else".into()
            } else {
                allow.join(", ")
            },
        });
    }
    let managed_rules = managed
        .forced_ask
        .iter()
        .map(|r| (ConflictKind::Ask, r))
        .chain(managed.forced_deny.iter().map(|r| (ConflictKind::Deny, r)));
    for (kind, rule) in managed_rules {
        let dot_rule = match allow.iter().find(|a| rules_overlap(a, rule)) {
            Some(a) => a.clone(),
            None if default_allows => "everything else".to_string(),
            None => continue,
        };
        out.push(Conflict {
            kind,
            rule: rule.clone(),
            dot_rule,
        });
    }
    out
}

/// `Tool(spec)` into (`Tool`, spec).
fn split_rule(rule: &str) -> (&str, Option<&str>) {
    let rule = rule.trim();
    match rule.split_once('(') {
        Some((tool, rest)) => (tool.trim(), Some(rest.trim_end_matches(')').trim())),
        None => (rule, None),
    }
}

/// The part of a pattern before its trailing wildcard (`git push:*` gives `git push`).
fn stem(p: &str) -> Option<&str> {
    p.strip_suffix(":*")
        .or_else(|| p.strip_suffix(" *"))
        .or_else(|| p.strip_suffix('*'))
}

/// Whether two patterns can match something in common, judged on their wildcard stems.
fn patterns_overlap(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (stem(a), stem(b)) {
        (Some(x), Some(y)) => x.starts_with(y) || y.starts_with(x),
        (Some(x), None) => b.starts_with(x),
        (None, Some(y)) => a.starts_with(y),
        (None, None) => false,
    }
}

/// Whether the two rules can apply to the same call. A rule without an argument covers every
/// call of its tool.
fn rules_overlap(a: &str, b: &str) -> bool {
    let (ta, sa) = split_rule(a);
    let (tb, sb) = split_rule(b);
    if !patterns_overlap(ta, tb) {
        return false;
    }
    match (sa, sb) {
        (Some(x), Some(y)) => patterns_overlap(x, y),
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::{Preset, Rule};

    fn dir_with(json: &str) -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("managed-settings.json"), json).unwrap();
        d
    }

    #[test]
    fn no_file_is_no_policy() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(read_from(d.path()), ManagedPolicy::default());
    }

    #[test]
    fn a_policy_that_blocks_bypass_mode() {
        let d = dir_with(r#"{"permissions":{"disableBypassPermissionsMode":"disable"}}"#);
        let m = read_from(d.path());
        assert!(m.blocks_skipping_asks);
        assert_eq!(m.path, Some(d.path().join("managed-settings.json")));
        assert_eq!(m.error, None);
    }

    #[test]
    fn ask_and_deny_rules_are_read() {
        let d = dir_with(
            r#"{"permissions":{"ask":["Bash(git push:*)"],"deny":["Read(./.env)","WebFetch"]}}"#,
        );
        let m = read_from(d.path());
        assert_eq!(m.forced_ask, ["Bash(git push:*)"]);
        assert_eq!(m.forced_deny, ["Read(./.env)", "WebFetch"]);
        assert!(!m.blocks_skipping_asks && !m.managed_rules_only);
    }

    #[test]
    fn managed_rules_only_is_read() {
        let d = dir_with(r#"{"allowManagedPermissionRulesOnly":true}"#);
        assert!(read_from(d.path()).managed_rules_only);
    }

    #[test]
    fn drop_ins_add_to_the_main_file() {
        let d = dir_with(r#"{"permissions":{"ask":["Bash"]}}"#);
        let sub = d.path().join("managed-settings.d");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(
            sub.join("10-a.json"),
            r#"{"permissions":{"deny":["WebFetch"]}}"#,
        )
        .unwrap();
        let m = read_from(d.path());
        assert_eq!(m.forced_ask, ["Bash"]);
        assert_eq!(m.forced_deny, ["WebFetch"]);
    }

    #[test]
    fn malformed_json_reports_the_path_and_does_not_crash() {
        let d = dir_with("{ nope");
        let m = read_from(d.path());
        let file = d.path().join("managed-settings.json");
        assert_eq!(m.path, Some(file.clone()));
        let e = m.error.unwrap();
        assert!(
            e.contains("Couldn't read") && e.contains(&file.display().to_string()),
            "{e}"
        );
        assert!(m.forced_ask.is_empty());
    }

    #[test]
    fn a_json_value_that_is_not_an_object_is_a_read_error() {
        let d = dir_with("[1]");
        assert!(read_from(d.path()).error.is_some());
    }

    fn managed(ask: &[&str], deny: &[&str], only: bool) -> ManagedPolicy {
        ManagedPolicy {
            path: Some("m.json".into()),
            forced_ask: ask.iter().map(|s| s.to_string()).collect(),
            forced_deny: deny.iter().map(|s| s.to_string()).collect(),
            managed_rules_only: only,
            ..Default::default()
        }
    }

    #[test]
    fn no_company_rules_no_conflict() {
        let p = Policy::preset(Preset::Trusted);
        assert!(conflicts(EngineKind::Claude, &p, &ManagedPolicy::default()).is_empty());
    }

    #[test]
    fn a_trusted_dot_conflicts_with_every_company_ask_and_deny() {
        let p = Policy::preset(Preset::Trusted);
        let m = managed(&["Bash(rm:*)"], &["WebFetch"], false);
        let c = conflicts(EngineKind::Claude, &p, &m);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].kind, ConflictKind::Ask);
        assert_eq!(c[0].rule, "Bash(rm:*)");
        assert_eq!(c[0].dot_rule, "everything else");
        assert_eq!(c[1].kind, ConflictKind::Deny);
    }

    #[test]
    fn a_dot_that_asks_by_default_conflicts_only_where_it_allows() {
        let p = Policy::preset(Preset::Sandboxed);
        let m = managed(&["Write(./**)", "Bash(rm:*)"], &[], false);
        let c = conflicts(EngineKind::Claude, &p, &m);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].rule, "Write(./**)");
        assert_eq!(c[0].dot_rule, "Write(./**)");
    }

    #[test]
    fn a_rule_without_an_argument_covers_the_tool() {
        let mut p = Policy::preset(Preset::ReadOnly);
        p.rules.push(Rule::new("Bash(git status:*)", Action::Allow));
        let c = conflicts(EngineKind::Claude, &p, &managed(&["Bash"], &[], false));
        assert_eq!(c.len(), 1);
        let m = managed(&["Bash(git push:*)"], &[], false);
        assert!(conflicts(EngineKind::Claude, &p, &m).is_empty());
    }

    #[test]
    fn mcp_wildcards_overlap() {
        let mut p = Policy::preset(Preset::ReadOnly);
        p.rules.push(Rule::new("mcp__github__*", Action::Allow));
        let m = managed(&[], &["mcp__github__delete_repo"], false);
        assert_eq!(conflicts(EngineKind::Claude, &p, &m).len(), 1);
        let m = managed(&[], &["mcp__slack__post"], false);
        assert!(conflicts(EngineKind::Claude, &p, &m).is_empty());
    }

    #[test]
    fn managed_rules_only_ignores_the_dots_allow_rules() {
        let p = Policy::preset(Preset::Sandboxed);
        let c = conflicts(EngineKind::Claude, &p, &managed(&[], &[], true));
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].kind, ConflictKind::ManagedRulesOnly);
    }

    #[test]
    fn a_dot_with_nothing_allowed_has_nothing_to_lose_to_managed_rules_only() {
        let p = Policy {
            preset: Preset::Custom,
            rules: vec![Rule::new("Bash", Action::Deny)],
            default: Action::Ask,
            mcp_default: Action::Ask,
        };
        assert!(conflicts(EngineKind::Claude, &p, &managed(&["Bash"], &[], true)).is_empty());
    }

    #[test]
    fn a_local_dot_is_not_affected() {
        let p = Policy::preset(Preset::Trusted);
        let m = managed(&["Bash"], &["Read"], true);
        assert!(conflicts(EngineKind::Local, &p, &m).is_empty());
    }

    #[test]
    fn blocking_bypass_mode_is_no_conflict() {
        let m = ManagedPolicy {
            blocks_skipping_asks: true,
            path: Some("m.json".into()),
            ..Default::default()
        };
        assert!(conflicts(EngineKind::Claude, &Policy::preset(Preset::Trusted), &m).is_empty());
    }

    #[test]
    fn check_reads_and_judges() {
        let d = dir_with(r#"{"permissions":{"ask":["Bash"]}}"#);
        let c = check(
            d.path(),
            EngineKind::Claude,
            &Policy::preset(Preset::Trusted),
        );
        assert_eq!(c.conflicts.len(), 1);
    }
}
