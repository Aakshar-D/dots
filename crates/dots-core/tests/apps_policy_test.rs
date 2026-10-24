//! The policy names `App` and `ProposeApp` (app library spec section 6).

mod common;

use dots_core::apps::policy_input;
use dots_core::policy::{Action, Policy, Preset, Rule, APP_TOOLS, PROPOSE_RULE};
use serde_json::json;

fn call(app: &str, tool: &str, writes: bool) -> serde_json::Value {
    policy_input(app, tool, writes, &json!({"id": "t1"}))
}

fn with_rules(preset: Preset, extra: &[(&str, Action)]) -> Policy {
    let mut p = Policy::preset(preset);
    p.rules.extend(extra.iter().map(|(t, a)| Rule::new(t, *a)));
    p
}

#[test]
fn every_preset_allows_apps_and_proposals() {
    for preset in [Preset::Sandboxed, Preset::Trusted, Preset::ReadOnly] {
        let p = Policy::preset(preset);
        assert_eq!(
            p.resolve("App", &call("projects", "update_task", true)),
            Action::Allow,
            "{preset:?}"
        );
        assert_eq!(
            p.resolve("ProposeApp", &json!({"path": "app"})),
            Action::Allow,
            "{preset:?}"
        );
    }
    assert_eq!(APP_TOOLS, ["App", "ProposeApp"]);
}

#[test]
fn an_ask_rule_wins_over_the_presets_allow() {
    let p = with_rules(Preset::Sandboxed, &[("App(projects.write)", Action::Ask)]);
    assert_eq!(
        p.resolve("App", &call("projects", "update_task", true)),
        Action::Ask
    );
    assert_eq!(
        p.resolve("App", &call("projects", "get_task", false)),
        Action::Allow
    );
    assert_eq!(
        p.resolve("App", &call("expenses", "add_expense", true)),
        Action::Allow
    );
    let p = with_rules(Preset::Sandboxed, &[("ProposeApp", Action::Ask)]);
    assert_eq!(p.resolve("ProposeApp", &json!({})), Action::Ask);
}

#[test]
fn a_rule_names_an_app_a_tool_or_the_writing_tools() {
    let p = with_rules(Preset::Sandboxed, &[("App(projects)", Action::Deny)]);
    assert_eq!(
        p.resolve("App", &call("projects", "get_task", false)),
        Action::Deny
    );
    assert_eq!(
        p.resolve("App", &call("expenses", "month_total", false)),
        Action::Allow
    );
    let p = with_rules(
        Preset::Sandboxed,
        &[("App(projects.get_task)", Action::Deny)],
    );
    assert_eq!(
        p.resolve("App", &call("projects", "get_task", false)),
        Action::Deny
    );
    assert_eq!(
        p.resolve("App", &call("projects", "list_tasks", false)),
        Action::Allow
    );
}

#[test]
fn a_call_without_an_app_matches_no_allow_rule() {
    let mut p = Policy::preset(Preset::Custom);
    p.default = Action::Ask;
    p.rules = vec![Rule::new("App(projects)", Action::Allow)];
    assert_eq!(p.resolve("App", &json!({"tool": "get_task"})), Action::Ask);
    p.rules = vec![Rule::new("App(projects)", Action::Deny)];
    assert_eq!(p.resolve("App", &json!({"tool": "get_task"})), Action::Deny);
}

#[test]
fn rules_outside_the_forms_are_refused() {
    for good in [
        "App",
        "App(projects)",
        "App(projects.get_task)",
        "App(projects.write)",
        "ProposeApp",
    ] {
        let p = with_rules(Preset::Custom, &[(good, Action::Ask)]);
        p.validate().unwrap_or_else(|e| panic!("{good}: {e}"));
    }
    for bad in [
        "App(Projects)",
        "App(projects.)",
        "App(projects.Get)",
        "App(a.b.c)",
        "App(*)",
    ] {
        let p = with_rules(Preset::Custom, &[(bad, Action::Ask)]);
        let err = p.validate().unwrap_err().to_string();
        assert!(err.contains("an App rule names an app"), "{bad}: {err}");
    }
    let p = with_rules(Preset::Custom, &[("ProposeApp(x)", Action::Ask)]);
    assert!(p.validate().unwrap_err().to_string().contains(PROPOSE_RULE));
}

#[test]
fn the_cli_never_sees_app_rules() {
    let p = with_rules(
        Preset::Sandboxed,
        &[
            ("App(projects)", Action::Deny),
            ("ProposeApp", Action::Deny),
        ],
    );
    let (allow, deny) = p.cli_lists();
    assert!(!allow
        .iter()
        .chain(&deny)
        .any(|t| t.starts_with("App") || t.starts_with("ProposeApp")));
}

#[test]
fn a_tool_named_write_is_refused() {
    let manifest =
        common::apps::MANIFEST.replace("\"name\": \"month_total\"", "\"name\": \"write\"");
    let err = dots_core::apps::manifest::Manifest::parse(&manifest)
        .unwrap_err()
        .to_string();
    assert!(
        err.contains(
            "the tool name \"write\" is taken: App(expenses.write) names the tools that write"
        ),
        "{err}"
    );
}

#[tokio::test]
async fn stored_dots_get_the_app_rules_once() {
    let (dir, store) = common::temp_store().await;
    let dot = store.create_dot(&common::spec("old")).await.unwrap();
    drop(store);
    // Make the dot look stored before apps: no app rules, and listed for the upgrade.
    let db = dir.path().join("dots.db");
    let pool = sqlx::SqlitePool::connect(&format!("sqlite:{}", db.display()))
        .await
        .unwrap();
    let stored: String = sqlx::query_scalar("SELECT policy FROM dots WHERE id = ?")
        .bind(&dot.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut policy: serde_json::Value = serde_json::from_str(&stored).unwrap();
    policy["rules"]
        .as_array_mut()
        .unwrap()
        .retain(|r| !APP_TOOLS.contains(&r["tool"].as_str().unwrap()));
    sqlx::query("UPDATE dots SET policy = ? WHERE id = ?")
        .bind(policy.to_string())
        .bind(&dot.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("CREATE TABLE upgrade_0013 (dot_id TEXT PRIMARY KEY)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO upgrade_0013 VALUES (?)")
        .bind(&dot.id)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;
    let store = dots_core::store::Store::open(&db).await.unwrap();
    let rules: Vec<String> = store
        .get_dot(&dot.id)
        .await
        .unwrap()
        .spec
        .policy
        .rules
        .iter()
        .map(|r| r.tool.clone())
        .collect();
    assert_eq!(rules.iter().filter(|t| t.as_str() == "App").count(), 1);
    assert_eq!(
        rules.iter().filter(|t| t.as_str() == "ProposeApp").count(),
        1
    );
    assert_eq!(rules.last().map(String::as_str), Some("ProposeApp"));
    drop(store);
    let store = dots_core::store::Store::open(&db).await.unwrap();
    assert_eq!(
        store
            .get_dot(&dot.id)
            .await
            .unwrap()
            .spec
            .policy
            .rules
            .len(),
        rules.len()
    );
}
