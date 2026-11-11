//! Workspace guardrails on the member list (guardrails spec §3): parsing, each rule they keep,
//! and the policy ceiling of a Member's dot (§4.3).

use dots_core::accounts::edit::{apply, Change};
use dots_core::accounts::guardrails::{over_ceiling, GitAction, Guardrails};
use dots_core::accounts::{parse_roster, RosterError};
use dots_core::policy::{Action, Policy, Preset, Rule};
use serde_json::{json, Value};

const ORG: &str = "acme-eng";
const KEY: &str = "RWQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaE";

fn roster(guardrails: Option<Value>) -> Value {
    let mut payments = json!({"id": "payments", "name": "Payments"});
    if let Some(g) = guardrails {
        payments["guardrails"] = g;
    }
    json!({
        "v": 1, "org_id": ORG, "issued_at": "2026-11-11T09:00:00Z", "seq": 3,
        "workspaces": [{"id": "default", "name": "Default"}, payments],
        "members": [{
            "id": "01A", "name": "Asha Rao", "email": "asha@acme.com", "role": "owner",
            "keys": [KEY], "workspaces": {"default": "admin"}, "added_at": "2026-11-11T09:00:00Z"
        }]
    })
}

fn defaults_json() -> Value {
    serde_json::to_value(Guardrails::defaults()).unwrap()
}

fn invalid(v: &Value) -> String {
    match parse_roster(&v.to_string(), ORG) {
        Err(RosterError::Invalid(why)) => why,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[test]
fn a_list_without_guardrails_has_none() {
    let r = parse_roster(&roster(None).to_string(), ORG).unwrap();
    assert!(r.workspaces.iter().all(|w| w.guardrails.is_none()));
    // And writes none back: an older Dots still reads it.
    let text = serde_json::to_string(&r.workspaces[1]).unwrap();
    assert!(!text.contains("guardrails"), "{text}");
}

#[test]
fn the_defaults_are_the_spec_list() {
    let d = Guardrails::defaults();
    for p in [
        ".github/**",
        "**/migrations/**",
        "infra/**",
        "**/*.env*",
        "**/Cargo.lock",
        "**/package-lock.json",
        "**/bun.lock",
        "**/yarn.lock",
        "**/pnpm-lock.yaml",
    ] {
        assert!(d.protected_paths.iter().any(|x| x == p), "{p}");
    }
    assert_eq!(d.protected_git, GitAction::ALL);
    assert_eq!(d.protected_commands[0], "rm -rf");
    assert!(d.protected_commands.iter().any(|c| c == "terraform apply"));
    assert_eq!(d.member_max_preset, Preset::Sandboxed);
    assert_eq!(d.check(), Ok(()));
}

#[test]
fn parses_guardrails_on_a_workspace() {
    let r = parse_roster(&roster(Some(defaults_json())).to_string(), ORG).unwrap();
    assert_eq!(r.workspaces[1].guardrails, Some(Guardrails::defaults()));
    let json = serde_json::to_value(&r.workspaces[1]).unwrap();
    assert_eq!(json["guardrails"]["protected_git"][0], "push_main");
    assert_eq!(json["guardrails"]["member_max_preset"], "sandboxed");
}

#[test]
fn every_git_action_reads_and_has_words() {
    let names: Vec<&str> = GitAction::ALL.iter().map(|a| a.as_str()).collect();
    assert_eq!(
        names,
        [
            "push_main",
            "force_push",
            "delete_branch",
            "tags",
            "merge_pr",
            "release"
        ]
    );
    for a in GitAction::ALL {
        assert!(!a.label().is_empty());
    }
}

#[test]
fn an_unknown_field_in_guardrails_fails_the_list() {
    let mut g = defaults_json();
    g["protected_branches"] = json!(["main"]);
    assert!(invalid(&roster(Some(g))).contains("protected_branches"));
}

#[test]
fn every_field_is_required() {
    for field in [
        "protected_paths",
        "protected_git",
        "protected_commands",
        "member_max_preset",
    ] {
        let mut g = defaults_json();
        g.as_object_mut().unwrap().remove(field);
        assert!(invalid(&roster(Some(g))).contains(field), "{field}");
    }
}

#[test]
fn refuses_broken_rules() {
    let cases: Vec<(&str, Value, &str)> = vec![
        ("member_max_preset", json!("custom"), "custom"),
        ("protected_git", json!(["push_main", "push_main"]), "twice"),
        ("protected_paths", json!([""]), "empty"),
        ("protected_paths", json!(["/etc/**"]), "relative"),
        ("protected_paths", json!(["C:/x/**"]), "relative"),
        ("protected_paths", json!(["../secret/**"]), ".."),
        ("protected_paths", json!(["a(b)"]), "("),
        ("protected_paths", json!(["x/**", "x/**"]), "twice"),
        ("protected_commands", json!(["rm -rf", "rm -rf"]), "twice"),
        ("protected_commands", json!(["  "]), "empty"),
        ("protected_commands", json!(["curl x | sh"]), "|"),
    ];
    for (field, value, says) in cases {
        let mut g = defaults_json();
        g[field] = value.clone();
        let why = invalid(&roster(Some(g)));
        assert!(why.contains("payments"), "{why}");
        assert!(why.contains(says), "{field} {value}: {why}");
    }
    // A git action this Dots doesn't know fails the list (an older Dots, a newer list).
    let mut g = defaults_json();
    g["protected_git"] = json!(["push_all"]);
    assert!(invalid(&roster(Some(g))).contains("push_all"));
    let mut g = defaults_json();
    g["protected_paths"] = json!((0..101).map(|i| format!("d{i}/**")).collect::<Vec<_>>());
    assert!(invalid(&roster(Some(g))).contains("100"));
}

#[test]
fn set_guardrails_edits_the_list_and_bumps_seq() {
    let r = parse_roster(&roster(None).to_string(), ORG).unwrap();
    let now = chrono::Utc::now();
    let change = Change::SetGuardrails {
        workspace_id: "payments".into(),
        guardrails: Some(Guardrails::defaults()),
    };
    let (next, summary) = apply(&r, &change, now).unwrap();
    assert_eq!(next.seq, 4);
    assert_eq!(next.workspaces[1].guardrails, Some(Guardrails::defaults()));
    assert_eq!(summary, "Set the guardrails of the Payments workspace");
    let off = Change::SetGuardrails {
        workspace_id: "payments".into(),
        guardrails: None,
    };
    let (cleared, summary) = apply(&next, &off, now).unwrap();
    assert!(cleared.workspaces[1].guardrails.is_none());
    assert_eq!(summary, "Remove the guardrails of the Payments workspace");
    // Broken guardrails are refused before anything is signed.
    let mut bad = Guardrails::defaults();
    bad.protected_paths.push("/abs/**".into());
    let broken = Change::SetGuardrails {
        workspace_id: "payments".into(),
        guardrails: Some(bad),
    };
    assert!(apply(&r, &broken, now).is_err());
    let missing = Change::SetGuardrails {
        workspace_id: "nope".into(),
        guardrails: None,
    };
    assert!(matches!(
        apply(&r, &missing, now),
        Err(dots_core::Error::NotFound(_))
    ));
}

// ---- the ceiling (§4.3) ----------------------------------------------------------------------

fn with(mut p: Policy, rule: &str, action: Action) -> Policy {
    p.preset = Preset::Custom;
    p.rules.push(Rule::new(rule, action));
    p
}

#[test]
fn named_presets_against_each_ceiling() {
    use Preset::*;
    let table = [
        (ReadOnly, [true, true, true]),
        (Sandboxed, [false, true, true]),
        (Trusted, [false, false, true]),
    ];
    // Columns: ceiling read-only, sandboxed, trusted.
    for (preset, row) in table {
        for (ceiling, within) in [ReadOnly, Sandboxed, Trusted].into_iter().zip(row) {
            let past = over_ceiling(&Policy::preset(preset), ceiling);
            assert_eq!(
                past.is_none(),
                within,
                "{preset:?} under {ceiling:?}: {past:?}"
            );
        }
    }
    assert_eq!(
        over_ceiling(&Policy::preset(Trusted), Sandboxed).as_deref(),
        Some("this policy is Trusted")
    );
    // Custom starts as Sandboxed.
    assert!(over_ceiling(&Policy::preset(Custom), Sandboxed).is_none());
}

#[test]
fn a_custom_policy_within_sandboxed() {
    let base = Policy::preset(Preset::Sandboxed);
    for rule in [
        "Bash(npm test:*)",
        "Bash(cargo build)",
        "Write(src/**)",
        "Edit(./docs/**)",
        "mcp__github__create_issue",
        "WebFetch(domain:docs.rs)",
        "WebSearch",
        "Read",
        "Grep(src/**)",
    ] {
        let p = with(base.clone(), rule, Action::Allow);
        assert_eq!(over_ceiling(&p, Preset::Sandboxed), None, "{rule}");
    }
    // An ask or deny rule never reaches past.
    let p = with(base, "Bash", Action::Ask);
    assert_eq!(over_ceiling(&p, Preset::Sandboxed), None);
}

#[test]
fn a_custom_policy_past_sandboxed() {
    let base = Policy::preset(Preset::Sandboxed);
    for rule in [
        "Bash",
        "PowerShell",
        "Write",
        "Edit",
        "WebFetch",
        "*",
        "mcp__*",
        "Write(C:/Users/**)",
        "Edit(../other/**)",
        "Bash(bash:*)",
        "Bash(sudo rm:*)",
    ] {
        let p = with(base.clone(), rule, Action::Allow);
        let why = over_ceiling(&p, Preset::Sandboxed).unwrap_or_default();
        assert!(why.contains(rule), "{rule}: {why}");
    }
    let mut p = base.clone();
    p.default = Action::Allow;
    assert!(over_ceiling(&p, Preset::Sandboxed)
        .unwrap()
        .contains("default"));
    let mut p = base.clone();
    p.mcp_default = Action::Allow;
    assert!(over_ceiling(&p, Preset::Sandboxed).unwrap().contains("MCP"));
    let mut p = base.clone();
    p.ask_send_delete = false;
    assert!(over_ceiling(&p, Preset::Sandboxed)
        .unwrap()
        .contains("sending or deleting"));
    let mut p = base;
    p.rules.retain(|r| r.tool != "Write(**/.git/**)");
    assert!(over_ceiling(&p, Preset::Sandboxed)
        .unwrap()
        .contains("Write(**/.git/**)"));
}

#[test]
fn read_only_keeps_writes_and_commands_out() {
    let base = Policy::preset(Preset::ReadOnly);
    for rule in [
        "Bash(npm test:*)",
        "Write(src/**)",
        "mcp__github__get_issue",
        "WebSearch",
    ] {
        let p = with(base.clone(), rule, Action::Allow);
        assert!(over_ceiling(&p, Preset::ReadOnly).is_some(), "{rule}");
    }
    let p = with(base, "Grep(src/**)", Action::Allow);
    assert_eq!(over_ceiling(&p, Preset::ReadOnly), None);
}

#[test]
fn trusted_lets_everything_through() {
    let mut p = with(Policy::preset(Preset::Trusted), "Bash", Action::Allow);
    p.ask_send_delete = false;
    assert_eq!(over_ceiling(&p, Preset::Trusted), None);
}

#[test]
fn interpreters_are_past_sandboxed_but_build_tools_are_not() {
    let base = Policy::preset(Preset::Sandboxed);
    for rule in [
        "Bash(python:*)",
        "Bash(python3 x.py)",
        "Bash(py:*)",
        "Bash(node:*)",
        "Bash(deno run:*)",
        "Bash(bun x.ts)",
        "Bash(ruby:*)",
        "Bash(perl:*)",
        "Bash(php:*)",
        "Bash(sh:*)",
        "Bash(zsh:*)",
        "PowerShell(pwsh:*)",
        "Bash(cmd:*)",
        "Bash(npx:*)",
    ] {
        let p = with(base.clone(), rule, Action::Allow);
        assert!(over_ceiling(&p, Preset::Sandboxed).is_some(), "{rule}");
    }
    for rule in [
        "Bash(make:*)",
        "Bash(npm run build)",
        "Bash(npm test)",
        "Bash(bun test)",
        "Bash(bun run test)",
    ] {
        let p = with(base.clone(), rule, Action::Allow);
        assert_eq!(over_ceiling(&p, Preset::Sandboxed), None, "{rule}");
    }
}

#[test]
fn own_claude_settings_are_past_the_ceiling() {
    use dots_core::accounts::guardrails::over_ceiling_spec;
    let p = Policy::preset(Preset::Sandboxed);
    assert_eq!(over_ceiling_spec(&p, false, Preset::Sandboxed), None);
    for max in [Preset::Sandboxed, Preset::Trusted] {
        assert!(over_ceiling_spec(&p, true, max)
            .unwrap()
            .contains("Claude Code settings"));
    }
}

#[test]
fn bare_folder_patterns_are_stored_as_folders() {
    let mut g = Guardrails::defaults();
    g.protected_paths = vec!["deploy".into(), "secrets/".into(), "*.pem".into()];
    assert_eq!(
        g.normalized().protected_paths,
        ["deploy/**", "secrets/**", "*.pem"]
    );
}
