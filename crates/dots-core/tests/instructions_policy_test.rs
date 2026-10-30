//! The policy name `Instructions` (conversational chat spec section 4).

mod common;

use dots_core::policy::{Action, Policy, Preset, Rule, INSTRUCTIONS_RULE, INSTRUCTIONS_TOOL};
use serde_json::{json, Value};

#[test]
fn every_preset_allows_proposing_instructions_last() {
    for preset in [
        Preset::Sandboxed,
        Preset::Trusted,
        Preset::ReadOnly,
        Preset::Custom,
    ] {
        let p = Policy::preset(preset);
        assert_eq!(
            p.resolve(INSTRUCTIONS_TOOL, &json!({})),
            Action::Allow,
            "{preset:?}"
        );
        assert_eq!(
            p.rules.last(),
            Some(&Rule::new("Instructions", Action::Allow))
        );
    }
}

#[test]
fn an_ask_or_deny_rule_wins_and_a_rule_names_no_argument() {
    let mut p = Policy::preset(Preset::Trusted);
    p.rules.push(Rule::new("Instructions", Action::Ask));
    assert_eq!(p.resolve("Instructions", &json!({})), Action::Ask);
    let mut p = Policy::preset(Preset::Trusted);
    p.rules.insert(0, Rule::new("Instructions", Action::Deny));
    assert_eq!(p.resolve("Instructions", &json!({})), Action::Deny);
    let mut p = Policy::preset(Preset::Sandboxed);
    p.rules
        .push(Rule::new("Instructions(anything)", Action::Allow));
    assert!(p
        .validate()
        .unwrap_err()
        .to_string()
        .contains(INSTRUCTIONS_RULE));
}

#[test]
fn the_cli_never_sees_the_instructions_rule() {
    let mut p = Policy::preset(Preset::Sandboxed);
    p.rules.push(Rule::new("Instructions", Action::Deny));
    let (allow, deny) = p.cli_lists();
    assert!(!allow
        .iter()
        .chain(&deny)
        .any(|t| t.starts_with("Instructions")));
}

/// Makes `dot_id` look stored before migration 0018: its policy without an `Instructions` rule
/// (or with `keep`), and listed in `upgrade_0018`.
async fn as_before_0018(db: &std::path::Path, dot_id: &str, keep: Option<Value>) {
    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db.display()))
        .await
        .unwrap();
    let stored: String = sqlx::query_scalar("SELECT policy FROM dots WHERE id = ?")
        .bind(dot_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut policy: Value = serde_json::from_str(&stored).unwrap();
    let rules = policy["rules"].as_array_mut().unwrap();
    rules.retain(|r| r["tool"] != "Instructions");
    if let Some(rule) = keep {
        rules.insert(0, rule);
    }
    sqlx::query("UPDATE dots SET policy = ? WHERE id = ?")
        .bind(policy.to_string())
        .bind(dot_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE IF NOT EXISTS upgrade_0018 (dot_id TEXT PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO upgrade_0018 VALUES (?)")
        .bind(dot_id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
}

#[tokio::test]
async fn stored_dots_get_the_instructions_rule_once_custom_too() {
    let (dir, store) = common::temp_store().await;
    let plain = store
        .create_dot(&common::spec("old"), None, "default")
        .await
        .unwrap();
    let mut custom = common::spec("custom");
    custom.policy = Policy::preset(Preset::Custom);
    let custom = store.create_dot(&custom, None, "default").await.unwrap();
    let denied = store
        .create_dot(&common::spec("denied"), None, "default")
        .await
        .unwrap();
    drop(store);
    let db = dir.path().join("dots.db");
    as_before_0018(&db, &plain.id, None).await;
    as_before_0018(&db, &custom.id, None).await;
    as_before_0018(
        &db,
        &denied.id,
        Some(json!({"tool": "Instructions", "action": "deny"})),
    )
    .await;
    let store = dots_core::store::Store::open(&db).await.unwrap();
    let rules = |id: String| {
        let store = store.clone();
        async move {
            store
                .get_dot(&id)
                .await
                .unwrap()
                .spec
                .policy
                .rules
                .into_iter()
                .filter(|r| r.tool == "Instructions")
                .map(|r| r.action)
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(rules(plain.id.clone()).await, vec![Action::Allow]);
    assert_eq!(rules(custom.id.clone()).await, vec![Action::Allow]);
    assert_eq!(rules(denied.id.clone()).await, vec![Action::Deny]);
    drop(store);
    let store = dots_core::store::Store::open(&db).await.unwrap();
    let again = store.get_dot(&plain.id).await.unwrap().spec.policy.rules;
    assert_eq!(again.iter().filter(|r| r.tool == "Instructions").count(), 1);
}
