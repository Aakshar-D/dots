//! The check of an MCP call (MCP spec section 4): `Policy::resolve_mcp`, `mcp_default`, the
//! presets' changes and the rules offered for an MCP tool.

use dots_core::policy::{allow_candidates, Action, McpTrust, Policy, Preset, Rule};
use serde_json::json;

const TRUSTED_SERVER: McpTrust = McpTrust {
    held: false,
    read_only: false,
};
const READ_ONLY_TOOL: McpTrust = McpTrust {
    held: false,
    read_only: true,
};
const HELD_SERVER: McpTrust = McpTrust {
    held: true,
    read_only: false,
};
const HELD_READ_ONLY_TOOL: McpTrust = McpTrust {
    held: true,
    read_only: true,
};

fn policy(rules: Vec<Rule>, mcp_default: Action) -> Policy {
    Policy {
        preset: Preset::Custom,
        rules,
        default: Action::Allow,
        mcp_default,
    }
}

fn resolve(p: &Policy, tool: &str, trust: McpTrust) -> Action {
    p.resolve_mcp(tool, &json!({}), trust)
}

const TOOL: &str = "mcp__sf__query";

#[test]
fn a_deny_rule_wins_over_everything() {
    let p = policy(
        vec![
            Rule::new("mcp__sf__*", Action::Deny),
            Rule::new(TOOL, Action::Allow),
        ],
        Action::Allow,
    );
    for trust in [TRUSTED_SERVER, READ_ONLY_TOOL, HELD_SERVER] {
        assert_eq!(resolve(&p, TOOL, trust), Action::Deny);
    }
}

#[test]
fn a_held_server_asks_even_for_an_allowed_or_read_only_tool() {
    let p = policy(vec![Rule::new(TOOL, Action::Allow)], Action::Allow);
    assert_eq!(resolve(&p, TOOL, HELD_SERVER), Action::Ask);
    assert_eq!(resolve(&p, TOOL, HELD_READ_ONLY_TOOL), Action::Ask);
    // Held beats the default too, and a deny default does not turn the ask into a deny.
    let denying = policy(vec![], Action::Deny);
    assert_eq!(resolve(&denying, TOOL, HELD_SERVER), Action::Ask);
}

#[test]
fn an_allow_rule_beats_an_ask_rule_and_the_default() {
    let p = policy(
        vec![
            Rule::new("mcp__sf__*", Action::Ask),
            Rule::new(TOOL, Action::Allow),
        ],
        Action::Deny,
    );
    assert_eq!(resolve(&p, TOOL, TRUSTED_SERVER), Action::Allow);
    assert_eq!(resolve(&p, "mcp__sf__update", TRUSTED_SERVER), Action::Ask);
}

#[test]
fn an_ask_rule_beats_the_read_only_flag() {
    let p = policy(vec![Rule::new("mcp__sf__*", Action::Ask)], Action::Allow);
    assert_eq!(resolve(&p, TOOL, READ_ONLY_TOOL), Action::Ask);
}

#[test]
fn a_read_only_tool_is_allowed_whatever_the_default() {
    for default in [Action::Allow, Action::Ask, Action::Deny] {
        let p = policy(vec![], default);
        assert_eq!(resolve(&p, TOOL, READ_ONLY_TOOL), Action::Allow);
    }
}

#[test]
fn any_other_tool_gets_mcp_default_not_default() {
    for mcp_default in [Action::Allow, Action::Ask, Action::Deny] {
        let p = policy(vec![], mcp_default);
        assert_eq!(resolve(&p, TOOL, TRUSTED_SERVER), mcp_default);
        // The plain check still uses `default`.
        assert_eq!(p.resolve(TOOL, &json!({})), Action::Allow);
    }
}

#[test]
fn the_presets_set_mcp_default() {
    for (preset, expected) in [
        (Preset::Sandboxed, Action::Ask),
        (Preset::Custom, Action::Ask),
        (Preset::Trusted, Action::Ask),
        (Preset::ReadOnly, Action::Deny),
    ] {
        assert_eq!(Policy::preset(preset).mcp_default, expected, "{preset:?}");
    }
}

#[test]
fn trusted_asks_for_a_writing_mcp_tool_and_allows_a_read_only_one() {
    let p = Policy::preset(Preset::Trusted);
    assert!(!p.rules.iter().any(|r| r.tool == "mcp__*"));
    assert_eq!(p.default, Action::Allow);
    assert_eq!(resolve(&p, TOOL, TRUSTED_SERVER), Action::Ask);
    assert_eq!(resolve(&p, TOOL, READ_ONLY_TOOL), Action::Allow);
    assert_eq!(resolve(&p, TOOL, HELD_SERVER), Action::Ask);
}

#[test]
fn read_only_denies_a_writing_mcp_tool_and_allows_a_read_only_one() {
    let p = Policy::preset(Preset::ReadOnly);
    assert_eq!(resolve(&p, TOOL, TRUSTED_SERVER), Action::Deny);
    assert_eq!(resolve(&p, TOOL, READ_ONLY_TOOL), Action::Allow);
}

#[test]
fn sandboxed_asks_for_a_writing_mcp_tool() {
    let p = Policy::preset(Preset::Sandboxed);
    assert_eq!(resolve(&p, TOOL, TRUSTED_SERVER), Action::Ask);
    assert_eq!(resolve(&p, TOOL, READ_ONLY_TOOL), Action::Allow);
}

#[test]
fn every_preset_allows_the_skill_tool() {
    for preset in [
        Preset::ReadOnly,
        Preset::Sandboxed,
        Preset::Trusted,
        Preset::Custom,
    ] {
        let p = Policy::preset(preset);
        assert!(p.cli_lists().0.contains(&"Skill".to_string()), "{preset:?}");
        assert_eq!(p.resolve("Skill", &json!({"name": "pdf"})), Action::Allow);
    }
}

#[test]
fn a_stored_policy_without_mcp_default_reads_as_ask() {
    let p: Policy = serde_json::from_value(json!({
        "preset": "custom",
        "rules": [{"tool": "Read", "action": "allow"}],
        "default": "allow",
    }))
    .unwrap();
    assert_eq!(p.mcp_default, Action::Ask);
    let back = serde_json::to_value(&p).unwrap();
    assert_eq!(back["mcp_default"], "ask");
}

#[test]
fn an_mcp_tool_offers_the_tool_and_its_server() {
    let found = allow_candidates("mcp__github__create_issue", &json!({"title": "x"}), None);
    let patterns: Vec<&str> = found.iter().map(|c| c.pattern.as_str()).collect();
    assert_eq!(
        patterns,
        vec!["mcp__github__create_issue", "mcp__github__*"]
    );
    assert_eq!(found[0].label, "every call of this tool");
    assert_eq!(found[1].label, "every tool of the server github");
    // Each one is a valid rule that allows the call.
    for c in &found {
        let p = policy(vec![Rule::new(&c.pattern, Action::Allow)], Action::Ask);
        p.validate().unwrap();
        assert_eq!(
            p.resolve_mcp("mcp__github__create_issue", &json!({}), TRUSTED_SERVER),
            Action::Allow
        );
    }
}

#[test]
fn a_name_without_a_server_and_a_tool_offers_only_itself() {
    for tool in ["mcp__github", "mcp__github__", "mcp____x"] {
        let found = allow_candidates(tool, &json!({}), None);
        let patterns: Vec<&str> = found.iter().map(|c| c.pattern.as_str()).collect();
        assert_eq!(patterns, vec![tool]);
    }
    assert!(allow_candidates("mcp__*", &json!({}), None).is_empty());
}
