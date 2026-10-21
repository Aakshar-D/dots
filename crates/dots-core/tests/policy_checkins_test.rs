//! The check-in tools in the policy (check-ins spec sections 4 and 8): every preset allows
//! `Notify`, `Ask` and `FollowUp` after `Skill` and `Context`, a dot's own rule still decides,
//! the CLI never sees their rules, and migration 0008 gives them to stored policies that are
//! not Custom.

use dots_core::policy::{Action, Policy, Preset, Rule, CHECK_IN_TOOLS};
use dots_core::store::Store;
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

#[test]
fn every_preset_allows_the_check_in_tools_after_skill_and_context() {
    assert_eq!(CHECK_IN_TOOLS, ["Notify", "Ask", "FollowUp"]);
    for preset in Preset::ALL {
        let p = Policy::preset(*preset);
        let tail: Vec<&str> = p.rules[p.rules.len() - 5..]
            .iter()
            .map(|r| r.tool.as_str())
            .collect();
        assert_eq!(
            tail,
            ["Skill", "Context", "Notify", "Ask", "FollowUp"],
            "{preset:?}"
        );
        for tool in CHECK_IN_TOOLS {
            assert_eq!(
                p.resolve(tool, &json!({"text": "x"})),
                Action::Allow,
                "{preset:?} {tool}"
            );
        }
        // The CLI has no such tools: their rules stay out of its lists.
        let (allow, deny) = p.cli_lists();
        assert!(
            !allow
                .iter()
                .chain(&deny)
                .any(|t| CHECK_IN_TOOLS.contains(&t.as_str())),
            "{preset:?}"
        );
    }
}

#[test]
fn a_dots_own_check_in_rule_still_decides_and_stays_out_of_the_cli_lists() {
    let mut p = Policy::preset(Preset::Sandboxed);
    p.rules.insert(0, Rule::new("Notify", Action::Deny));
    assert_eq!(p.resolve("Notify", &json!({"text": "x"})), Action::Deny);
    // deny > allow > ask: an ask rule decides once the preset's allow is gone.
    p.rules.retain(|r| r.tool != "FollowUp");
    p.rules.push(Rule::new("FollowUp", Action::Ask));
    assert_eq!(
        p.resolve("FollowUp", &json!({"note": "x", "in": "1h"})),
        Action::Ask
    );
    p.rules.push(Rule::new("Ask(x)", Action::Deny));
    assert!(p.validate().is_ok());
    let (allow, deny) = p.cli_lists();
    assert!(
        !deny.iter().any(|t| t == "Notify" || t == "Ask(x)"),
        "{deny:?}"
    );
    assert!(!allow.iter().any(|t| t == "Ask"), "{allow:?}");
    // A tool whose name only starts like one stays the CLI's.
    p.rules.push(Rule::new("Asks", Action::Deny));
    assert!(p.cli_lists().1.contains(&"Asks".to_string()));
}

/// Migration 0008 on a database Dots left at 0007: every stored policy whose preset is not
/// Custom gets an allow rule for each check-in tool that no rule names, after `Context`.
#[tokio::test]
async fn migration_0008_lets_every_policy_that_is_not_custom_check_in() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version < 8)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    // A policy as 0007 left it: the preset's rules without the check-in tools.
    let old = |preset: Preset| {
        let mut p = Policy::preset(preset);
        p.rules
            .retain(|r| !CHECK_IN_TOOLS.contains(&r.tool.as_str()));
        p
    };
    let mut asks = old(Preset::Trusted);
    asks.rules.push(Rule::new("Ask", Action::Deny));
    let custom = old(Preset::Custom);
    for (id, name, policy) in [
        ("D1", "sandboxed", old(Preset::Sandboxed)),
        ("D2", "read-only", old(Preset::ReadOnly)),
        ("D3", "trusted", asks.clone()),
        ("D4", "custom", custom.clone()),
    ] {
        sqlx::query(
            "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
             webhook_token, policy, max_turns, timeout_secs, approval_wait_secs, \
             created_at, updated_at) VALUES (?, ?, 'Do it.', 'claude', 'sonnet', 'C:/tmp/repo', \
             'worktree', 't', ?, 40, 1800, 240, '2026-10-01T00:00:00.000Z', \
             '2026-10-01T00:00:00.000Z')",
        )
        .bind(id)
        .bind(name)
        .bind(serde_json::to_string(&policy).unwrap())
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;

    let store = Store::open(&path).await.unwrap();
    for (id, preset) in [("D1", Preset::Sandboxed), ("D2", Preset::ReadOnly)] {
        assert_eq!(
            store.get_dot(id).await.unwrap().spec.policy,
            Policy::preset(preset),
            "{id}"
        );
    }
    // A rule that names a tool already is the person's: only the others are added.
    let mut want = asks;
    want.rules.push(Rule::new("Notify", Action::Allow));
    want.rules.push(Rule::new("FollowUp", Action::Allow));
    assert_eq!(store.get_dot("D3").await.unwrap().spec.policy, want);
    // A Custom policy is the person's own: it is left as it was.
    assert_eq!(store.get_dot("D4").await.unwrap().spec.policy, custom);
    store.close().await;
    // Opening again adds nothing twice.
    let store = Store::open(&path).await.unwrap();
    assert_eq!(
        store.get_dot("D1").await.unwrap().spec.policy,
        Policy::preset(Preset::Sandboxed)
    );
}
