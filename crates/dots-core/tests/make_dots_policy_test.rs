//! The policy name `MakeDots` (dots-make-dots design section 5): every preset allows it last, an
//! ask or deny rule the person adds wins over that allow, a rule names it alone, the CLI never
//! sees its rules, and migration 0048 gives it to stored policies, Custom too.

use dots_core::policy::{Action, Policy, Preset, Rule, MAKE_DOTS_RULE, MAKE_DOTS_TOOL};
use dots_core::store::Store;
use serde_json::json;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

#[test]
fn every_preset_allows_making_dots_last() {
    assert_eq!(MAKE_DOTS_TOOL, "MakeDots");
    for preset in Preset::ALL {
        let p = Policy::preset(*preset);
        assert_eq!(
            p.resolve(MAKE_DOTS_TOOL, &json!({})),
            Action::Allow,
            "{preset:?}"
        );
        assert_eq!(
            p.rules.last(),
            Some(&Rule::new("MakeDots", Action::Allow)),
            "{preset:?}"
        );
        p.validate().unwrap();
    }
}

#[test]
fn an_ask_or_deny_rule_wins_and_a_rule_names_no_argument() {
    // The person's ask wins over the preset's allow, in a Custom policy and in any other.
    for preset in Preset::ALL {
        let mut p = Policy::preset(*preset);
        p.rules.push(Rule::new("MakeDots", Action::Ask));
        assert_eq!(p.resolve("MakeDots", &json!({})), Action::Ask, "{preset:?}");
        // Whatever the call carries: no call of it is exempt from asking.
        assert_eq!(
            p.resolve("MakeDots", &json!({"tool": "give_job"})),
            Action::Ask,
            "{preset:?}"
        );
        p.validate().unwrap();
    }
    let mut p = Policy::preset(Preset::Trusted);
    p.rules.insert(0, Rule::new("MakeDots", Action::Deny));
    assert_eq!(p.resolve("MakeDots", &json!({})), Action::Deny);
    // A deny wins over an ask as well.
    p.rules.push(Rule::new("MakeDots", Action::Ask));
    assert_eq!(p.resolve("MakeDots", &json!({})), Action::Deny);
    for action in [Action::Allow, Action::Ask, Action::Deny] {
        let mut p = Policy::preset(Preset::Sandboxed);
        p.rules.push(Rule::new("MakeDots(anything)", action));
        let e = p.validate().unwrap_err().to_string();
        assert!(e.contains(MAKE_DOTS_RULE), "{e}");
    }
}

#[test]
fn the_cli_never_sees_the_make_dots_rule() {
    for preset in Preset::ALL {
        let (allow, deny) = Policy::preset(*preset).cli_lists();
        assert!(
            !allow.iter().chain(&deny).any(|t| t.starts_with("MakeDots")),
            "{preset:?}"
        );
    }
    let mut p = Policy::preset(Preset::Sandboxed);
    p.rules.push(Rule::new("MakeDots", Action::Deny));
    p.rules.push(Rule::new(" MakeDots ", Action::Deny));
    let (allow, deny) = p.cli_lists();
    assert!(!allow.iter().chain(&deny).any(|t| t.starts_with("MakeDots")));
    // A tool whose name only starts like it stays the CLI's.
    let mut p = Policy::preset(Preset::Sandboxed);
    p.rules.push(Rule::new("MakeDotsLater", Action::Deny));
    assert!(p.cli_lists().1.contains(&"MakeDotsLater".to_string()));
}

/// Migration 0048 on a database Dots left at 0047: every stored policy, a Custom one too, gets
/// a `MakeDots` allow rule at the end unless a rule names it; opening again adds nothing.
#[tokio::test]
async fn migration_0048_gives_every_stored_policy_the_make_dots_rule_once() {
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
        .filter(|m| m.version < 48)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    // A policy as 0047 left it: the preset's rules without `MakeDots`.
    let old = |preset: Preset| {
        let mut p = Policy::preset(preset);
        p.rules.retain(|r| r.tool != "MakeDots");
        p
    };
    // A Custom policy the person edited, and one where they already decided on `MakeDots`.
    let mut custom = old(Preset::Custom);
    custom
        .rules
        .push(Rule::new("Bash(cargo test:*)", Action::Allow));
    let mut denies = old(Preset::Custom);
    denies.rules.insert(0, Rule::new("MakeDots", Action::Deny));
    let mut asks = old(Preset::Trusted);
    asks.rules.insert(0, Rule::new("MakeDots", Action::Ask));
    for (id, name, policy) in [
        ("D1", "sandboxed", old(Preset::Sandboxed)),
        ("D2", "read-only", old(Preset::ReadOnly)),
        ("D3", "trusted", old(Preset::Trusted)),
        ("D4", "custom", custom.clone()),
        ("D5", "denies", denies.clone()),
        ("D6", "asks", asks.clone()),
    ] {
        sqlx::query(
            "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
             webhook_token, policy, max_turns, timeout_secs, approval_wait_secs, \
             created_at, updated_at) VALUES (?, ?, 'Do it.', 'claude', 'sonnet', 'C:/tmp/repo', \
             'worktree', '', ?, 40, 1800, 240, '2026-10-01T00:00:00.000Z', \
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
    let policy = |id: &'static str| {
        let store = store.clone();
        async move { store.get_dot(id).await.unwrap().spec.policy }
    };
    for (id, preset) in [
        ("D1", Preset::Sandboxed),
        ("D2", Preset::ReadOnly),
        ("D3", Preset::Trusted),
    ] {
        let p = policy(id).await;
        assert_eq!(p, Policy::preset(preset), "{id}");
        assert_eq!(p.resolve("MakeDots", &json!({})), Action::Allow, "{id}");
    }
    // A Custom policy gets the rule too, at the end, and is otherwise as the person made it.
    let mut custom_then = custom.clone();
    custom_then.rules.push(Rule::new("MakeDots", Action::Allow));
    assert_eq!(policy("D4").await, custom_then);
    assert_eq!(custom_then.preset, Preset::Custom);
    // A rule that names `MakeDots` already is the person's: nothing is added.
    assert_eq!(policy("D5").await, denies);
    assert_eq!(policy("D6").await, asks);
    assert_eq!(asks.resolve("MakeDots", &json!({})), Action::Ask);
    // The list of dots to do is gone.
    let pool = SqlitePool::connect(&format!("sqlite:{}", path.display()))
        .await
        .unwrap();
    let waiting = |pool: SqlitePool| async move {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'upgrade_0048'",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    assert_eq!(waiting(pool.clone()).await, 0);
    pool.close().await;
    store.close().await;

    // Opening again changes nothing.
    let store = Store::open(&path).await.unwrap();
    for (id, want) in [
        ("D1", Policy::preset(Preset::Sandboxed)),
        ("D2", Policy::preset(Preset::ReadOnly)),
        ("D3", Policy::preset(Preset::Trusted)),
        ("D4", custom_then),
        ("D5", denies),
        ("D6", asks),
    ] {
        assert_eq!(store.get_dot(id).await.unwrap().spec.policy, want, "{id}");
    }
}

/// A dot made after the upgrade has the rule from its preset, and the upgrade's list is empty
/// for a new database.
#[tokio::test]
async fn a_new_database_has_nothing_waiting_and_new_dots_allow_making() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let store = Store::open(&path).await.unwrap();
    let waiting: i64 = {
        let pool = SqlitePool::connect(&format!("sqlite:{}", path.display()))
            .await
            .unwrap();
        let n = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'upgrade_0048'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        pool.close().await;
        n
    };
    assert_eq!(waiting, 0);
    let spec = dots_core::model::DotSpec::new("fresh", "Do it.", dir.path().to_str().unwrap());
    let dot = store.create_dot(&spec, None, "default").await.unwrap();
    assert_eq!(
        dot.spec.policy.resolve("MakeDots", &json!({})),
        Action::Allow
    );
}
