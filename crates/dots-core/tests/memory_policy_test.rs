//! `Memory` in the policy (dot memory spec section 3): every preset allows it after the
//! check-in tools, an ask or deny rule the person adds wins over that allow, a rule names it
//! alone, the CLI never sees its rules, and migration 0012 gives it to stored policies that are
//! not Custom.

use dots_core::policy::{Action, Policy, Preset, Rule, CHECK_IN_TOOLS, MEMORY_RULE, MEMORY_TOOL};
use dots_core::store::Store;
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

#[test]
fn every_preset_allows_memory_after_the_check_in_tools() {
    assert_eq!(MEMORY_TOOL, "Memory");
    for preset in Preset::ALL {
        let p = Policy::preset(*preset);
        // `Workflow` follows it (teams spec section 7).
        let tail: Vec<&str> = p.rules[p.rules.len() - 5..]
            .iter()
            .map(|r| r.tool.as_str())
            .collect();
        assert_eq!(
            tail,
            ["Notify", "Ask", "FollowUp", "Memory", "Workflow"],
            "{preset:?}"
        );
        for input in [json!({"text": "x"}), json!({"id": "x"}), json!({})] {
            assert_eq!(p.resolve("Memory", &input), Action::Allow, "{preset:?}");
        }
        let (allow, deny) = p.cli_lists();
        assert!(
            !allow.iter().chain(&deny).any(|t| t == "Memory"),
            "{preset:?}"
        );
    }
}

#[test]
fn an_ask_or_deny_rule_on_memory_wins_over_the_presets_allow() {
    for preset in Preset::ALL {
        let mut p = Policy::preset(*preset);
        p.rules.push(Rule::new("Memory", Action::Ask));
        assert_eq!(
            p.resolve("Memory", &json!({"text": "x"})),
            Action::Ask,
            "{preset:?}"
        );
        p.rules.push(Rule::new("Memory", Action::Deny));
        assert_eq!(
            p.resolve("Memory", &json!({"text": "x"})),
            Action::Deny,
            "{preset:?}"
        );
        assert!(p.validate().is_ok());
        let (allow, deny) = p.cli_lists();
        assert!(
            !allow.iter().chain(&deny).any(|t| t == "Memory"),
            "{deny:?}"
        );
    }
    // A rule names `Memory` alone: the three tools are not separate rule names.
    let mut p = Policy::preset(Preset::Sandboxed);
    p.rules.push(Rule::new("Memory(remember)", Action::Deny));
    let e = p.validate().unwrap_err().to_string();
    assert!(e.contains(MEMORY_RULE), "{e}");
    // The person reads it as it is: no Markdown.
    assert!(!e.contains('`'), "{e}");
    // A tool whose name only starts like it stays the CLI's.
    let mut p = Policy::preset(Preset::Sandboxed);
    p.rules.push(Rule::new("Memoryless", Action::Deny));
    assert!(p.cli_lists().1.contains(&"Memoryless".to_string()));
}

/// Migration 0012 on a database Dots left at 0010: every stored policy whose preset is not
/// Custom gets a `Memory` allow rule unless a rule names it, after the check-in tools.
#[tokio::test]
async fn migration_0012_gives_every_policy_that_is_not_custom_its_memory() {
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
        .filter(|m| m.version < 12)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    // A policy as 0010 left it: the preset's rules without `Memory`, and without `Workflow`,
    // which 0015 adds after it.
    let old = |preset: Preset| {
        let mut p = Policy::preset(preset);
        p.rules
            .retain(|r| r.tool != "Memory" && r.tool != "Workflow");
        p
    };
    let mut asks = old(Preset::Trusted);
    asks.rules.push(Rule::new("Memory", Action::Ask));
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
    // The person's own rule is kept, and no allow is added for it (0015 adds `Workflow`).
    let mut kept = asks;
    kept.rules.push(Rule::new("Workflow", Action::Allow));
    assert_eq!(store.get_dot("D3").await.unwrap().spec.policy, kept);
    assert_eq!(store.get_dot("D4").await.unwrap().spec.policy, custom);
    assert!(CHECK_IN_TOOLS
        .iter()
        .all(|t| custom.rules.iter().any(|r| r.tool == *t)));
    store.close().await;
    // Opening again adds nothing twice.
    let store = Store::open(&path).await.unwrap();
    assert_eq!(
        store.get_dot("D1").await.unwrap().spec.policy,
        Policy::preset(Preset::Sandboxed)
    );
}
