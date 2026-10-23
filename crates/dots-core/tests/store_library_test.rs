//! The Library in the store: items, catalogs and the dots that use them (MCP/skills spec
//! sections 2, 3 and 10), and migration 0006.

mod common;

use std::collections::BTreeMap;

use common::{spec, temp_store};
use dots_core::library::{
    CatalogStatus, LibraryConfig, LibraryKind, McpCatalog, McpServerConfig, PluginConfig,
    SkillConfig, Uses,
};
use dots_core::policy::{Action, Policy, Preset, Rule};
use dots_core::store::Store;
use dots_core::Error;
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};

fn server(command: &str) -> LibraryConfig {
    LibraryConfig::McpServer(McpServerConfig::Stdio {
        command: command.into(),
        args: Vec::new(),
        cwd: None,
        env: BTreeMap::new(),
    })
}

fn skill(path: &str) -> LibraryConfig {
    LibraryConfig::Skill(SkillConfig { path: path.into() })
}

fn plugin(path: &str) -> LibraryConfig {
    LibraryConfig::Plugin(PluginConfig {
        path: path.into(),
        installed_id: None,
        run_hooks: false,
    })
}

fn uses(mcp: &[&str], skills: &[&str], plugins: &[&str]) -> Uses {
    let v = |names: &[&str]| names.iter().map(|s| s.to_string()).collect();
    Uses {
        mcp_servers: v(mcp),
        skills: v(skills),
        plugins: v(plugins),
        context: Vec::new(),
    }
}

#[tokio::test]
async fn items_are_added_read_and_listed() {
    let (_d, store) = temp_store().await;
    let sf = store
        .add_library_item("sf", &server("sf-mcp"), None)
        .await
        .unwrap();
    assert_eq!(sf.id.len(), 26);
    assert_eq!(sf.kind(), LibraryKind::McpServer);
    assert_eq!(sf.config, server("sf-mcp"));
    assert_eq!(sf.created_at, sf.updated_at);
    assert_eq!(store.get_library_item(&sf.id).await.unwrap(), sf);
    // The same name may be used once per kind.
    let pdf_skill = store
        .add_library_item("pdf", &skill("C:/skills/pdf"), None)
        .await
        .unwrap();
    let pdf_plugin = store
        .add_library_item("pdf", &plugin("C:/plugins/pdf"), None)
        .await
        .unwrap();
    let err = store
        .add_library_item("sf", &server("other"), None)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    assert_eq!(
        store
            .find_library_item(LibraryKind::Skill, "pdf")
            .await
            .unwrap(),
        Some(pdf_skill.clone())
    );
    assert_eq!(
        store
            .find_library_item(LibraryKind::McpServer, "pdf")
            .await
            .unwrap(),
        None
    );
    // By kind (servers, skills, plugins), then by name.
    let all = store.list_library_items().await.unwrap();
    assert_eq!(all, vec![sf, pdf_skill, pdf_plugin]);
    let err = store.get_library_item("nope").await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn bad_names_and_configs_are_refused() {
    let (_d, store) = temp_store().await;
    for (name, config) in [
        ("a__b", server("x")),
        ("dots", server("x")),
        ("Upper", server("x")),
        ("sf", server(" ")),
        ("../x", skill("C:/s")),
        ("p", plugin("")),
    ] {
        let err = store
            .add_library_item(name, &config, None)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{name}: {err:?}");
    }
    // Only an MCP server belongs to an item, and only to a plugin.
    let sk = store
        .add_library_item("pdf", &skill("C:/s"), None)
        .await
        .unwrap();
    let err = store
        .add_library_item("plugin-pdf-x", &server("x"), Some(&sk.id))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    let pl = store
        .add_library_item("tools", &plugin("C:/p"), None)
        .await
        .unwrap();
    let err = store
        .add_library_item("other", &skill("C:/o"), Some(&pl.id))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    assert!(store
        .add_library_item("plugin-tools-x", &server("x"), Some(&pl.id))
        .await
        .is_ok());
}

#[tokio::test]
async fn a_new_server_has_an_unchecked_catalog() {
    let (_d, store) = temp_store().await;
    let sf = store
        .add_library_item("sf", &server("sf-mcp"), None)
        .await
        .unwrap();
    assert_eq!(
        store.get_catalog(&sf.id).await.unwrap(),
        McpCatalog::unchecked(&sf.id)
    );
    let sk = store
        .add_library_item("pdf", &skill("C:/s"), None)
        .await
        .unwrap();
    let err = store.get_catalog(&sk.id).await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    assert_eq!(store.list_catalogs().await.unwrap().len(), 1);
}

#[tokio::test]
async fn a_catalog_is_written_and_accepted_only_at_the_seen_fingerprint() {
    let (_d, store) = temp_store().await;
    let sf = store
        .add_library_item("sf", &server("sf-mcp"), None)
        .await
        .unwrap();
    let tools = json!([{"name": "query", "annotations": {"readOnlyHint": true}}]);
    let seen = McpCatalog {
        seen_fingerprint: Some("f1".into()),
        seen_tools: Some(tools.clone()),
        status: CatalogStatus::Changed,
        checked_at: Some("2026-10-15T00:00:00.000Z".into()),
        ..McpCatalog::unchecked(&sf.id)
    };
    store.put_catalog(&seen).await.unwrap();
    assert_eq!(store.get_catalog(&sf.id).await.unwrap(), seen);

    let err = store.accept_catalog(&sf.id, "f0").await.unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    let accepted = store.accept_catalog(&sf.id, "f1").await.unwrap();
    assert_eq!(accepted.status, CatalogStatus::Accepted);
    assert_eq!(accepted.accepted_fingerprint.as_deref(), Some("f1"));
    assert_eq!(accepted.accepted_tools, Some(tools));
    let err = store.accept_catalog("nope", "f1").await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn changing_a_server_config_resets_its_catalog() {
    let (_d, store) = temp_store().await;
    let sf = store
        .add_library_item("sf", &server("sf-mcp"), None)
        .await
        .unwrap();
    store
        .put_catalog(&McpCatalog {
            seen_fingerprint: Some("f1".into()),
            seen_tools: Some(json!([])),
            status: CatalogStatus::Changed,
            ..McpCatalog::unchecked(&sf.id)
        })
        .await
        .unwrap();
    store.accept_catalog(&sf.id, "f1").await.unwrap();

    // The same config: nothing to check again.
    store
        .update_library_item(&sf.id, &server("sf-mcp"))
        .await
        .unwrap();
    assert_eq!(
        store.get_catalog(&sf.id).await.unwrap().status,
        CatalogStatus::Accepted
    );

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let changed = store
        .update_library_item(&sf.id, &server("sf-mcp-2"))
        .await
        .unwrap();
    assert_eq!(changed.config, server("sf-mcp-2"));
    assert!(changed.updated_at > sf.updated_at);
    let catalog = store.get_catalog(&sf.id).await.unwrap();
    assert_eq!(catalog.status, CatalogStatus::Unchecked);
    assert_eq!(catalog.seen_fingerprint, None);
    // The list accepted before stays, for the review of the next check.
    assert_eq!(catalog.accepted_fingerprint.as_deref(), Some("f1"));

    let err = store
        .update_library_item(&sf.id, &skill("C:/s"))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    let err = store
        .update_library_item(&sf.id, &server(""))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    let err = store
        .update_library_item("nope", &server("x"))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn a_dot_uses_only_what_the_library_has() {
    let (_d, store) = temp_store().await;
    let tools = store
        .add_library_item("tools", &plugin("C:/p"), None)
        .await
        .unwrap();
    store
        .add_library_item("plugin-tools-files", &server("f"), Some(&tools.id))
        .await
        .unwrap();
    store
        .add_library_item("sf", &server("sf-mcp"), None)
        .await
        .unwrap();
    store
        .add_library_item("pdf", &skill("C:/s"), None)
        .await
        .unwrap();

    let mut s = spec("triage");
    s.uses = uses(&["sf"], &["pdf"], &["tools"]);
    let dot = store.create_dot(&s).await.unwrap();
    assert_eq!(dot.spec.uses, s.uses);

    for (name, bad) in [
        ("srv", uses(&["nope"], &[], &[])),
        ("skl", uses(&[], &["sf"], &[])),
        ("plg", uses(&[], &[], &["pdf"])),
        // A plugin's server comes with the plugin; it is not ticked on its own.
        ("own", uses(&["plugin-tools-files"], &[], &[])),
        ("dup", uses(&["sf", "sf"], &[], &[])),
    ] {
        let mut s = spec(name);
        s.uses = bad;
        let err = store.create_dot(&s).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{name}: {err:?}");
        let mut changed = dot.spec.clone();
        changed.uses = s.uses.clone();
        let err = store.update_dot(&dot.id, &changed).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{name}: {err:?}");
    }
    let names: Vec<String> = store
        .list_dots()
        .await
        .unwrap()
        .into_iter()
        .map(|d| d.spec.name)
        .collect();
    assert_eq!(names, ["triage"]);
    assert_eq!(store.get_dot(&dot.id).await.unwrap().spec.uses, s.uses);
}

#[tokio::test]
async fn a_run_gets_the_ticked_servers_and_those_of_the_ticked_plugins() {
    let (_d, store) = temp_store().await;
    let tools = store
        .add_library_item("tools", &plugin("C:/p"), None)
        .await
        .unwrap();
    let files = store
        .add_library_item("plugin-tools-files", &server("f"), Some(&tools.id))
        .await
        .unwrap();
    let sf = store
        .add_library_item("sf", &server("sf-mcp"), None)
        .await
        .unwrap();
    store
        .add_library_item("gh", &server("gh-mcp"), None)
        .await
        .unwrap();
    let got = store
        .used_mcp_servers(&uses(&["sf"], &[], &["tools"]))
        .await
        .unwrap();
    assert_eq!(got, vec![files.clone(), sf.clone()]);
    assert_eq!(
        store
            .used_mcp_servers(&uses(&["sf"], &[], &[]))
            .await
            .unwrap(),
        vec![sf]
    );
    assert!(store
        .used_mcp_servers(&Uses::default())
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn removing_an_item_unticks_it_and_takes_a_plugins_servers_along() {
    let (_d, store) = temp_store().await;
    let tools = store
        .add_library_item("tools", &plugin("C:/p"), None)
        .await
        .unwrap();
    let files = store
        .add_library_item("plugin-tools-files", &server("f"), Some(&tools.id))
        .await
        .unwrap();
    let sf = store
        .add_library_item("sf", &server("sf-mcp"), None)
        .await
        .unwrap();
    let mut a = spec("a");
    a.uses = uses(&["sf"], &[], &["tools"]);
    let a = store.create_dot(&a).await.unwrap();
    let mut b = spec("b");
    b.uses = uses(&["sf"], &[], &[]);
    let b = store.create_dot(&b).await.unwrap();
    let c = store.create_dot(&spec("c")).await.unwrap();

    let names = |dots: Vec<dots_core::model::Dot>| -> Vec<String> {
        dots.into_iter().map(|d| d.spec.name).collect()
    };
    assert_eq!(names(store.dots_using(&sf).await.unwrap()), ["a", "b"]);
    assert_eq!(names(store.dots_using(&tools).await.unwrap()), ["a"]);
    // A plugin's server is used by the dots that use the plugin.
    assert_eq!(names(store.dots_using(&files).await.unwrap()), ["a"]);

    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    store.remove_library_item(&sf.id).await.unwrap();
    let a2 = store.get_dot(&a.id).await.unwrap();
    assert_eq!(a2.spec.uses, uses(&[], &[], &["tools"]));
    assert!(a2.updated_at > a.updated_at);
    assert_eq!(
        store.get_dot(&b.id).await.unwrap().spec.uses,
        Uses::default()
    );
    assert_eq!(store.get_dot(&c.id).await.unwrap(), c);
    let err = store.get_catalog(&sf.id).await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");

    store.remove_library_item(&tools.id).await.unwrap();
    assert_eq!(
        store.get_dot(&a.id).await.unwrap().spec.uses,
        Uses::default()
    );
    let err = store.get_library_item(&files.id).await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    assert!(store.list_catalogs().await.unwrap().is_empty());
    let err = store.remove_library_item(&tools.id).await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}

/// A database as Dots left it at migration 0005, with the dots of `rows`: id, name, policy
/// JSON and the old `mcp_servers` JSON.
async fn database_0005(path: &std::path::Path, rows: &[(&str, &str, Value, Option<Value>)]) {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version <= 5)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    for (i, (id, name, policy, mcp)) in rows.iter().enumerate() {
        sqlx::query(
            "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
             webhook_token, policy, mcp_servers, max_turns, timeout_secs, approval_wait_secs, \
             created_at, updated_at) VALUES (?, ?, 'Do it.', 'claude', 'sonnet', 'C:/tmp/repo', \
             'worktree', 't', ?, ?, 40, 1800, 240, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(policy.to_string())
        .bind(mcp.as_ref().map(|v| v.to_string()))
        .bind(format!("2026-10-01T00:00:0{i}.000Z"))
        .bind(format!("2026-10-01T00:00:0{i}.000Z"))
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;
}

/// A policy as stored before `mcp_default` existed.
fn old_policy(preset: &str, rules: Value, default: &str) -> Value {
    json!({"preset": preset, "rules": rules, "default": default})
}

/// Migration 0006 on a database with per-dot MCP JSON (spec section 10).
#[tokio::test]
async fn migration_0006_moves_each_dots_servers_into_the_library() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let sandboxed = || {
        old_policy(
            "sandboxed",
            json!([{"tool": "Read", "action": "allow"}]),
            "ask",
        )
    };
    let sf = json!({"type": "stdio", "command": "sf-mcp", "args": ["--org", "acme"],
                    "env": {"SF_TOKEN": "abc", "MODE": "readonly", "API_KEY": "${MY_KEY}"}});
    database_0005(
        &path,
        &[
            // D1 and D2 have the same server: one item.
            (
                "D1",
                "one",
                old_policy(
                    "custom",
                    json!([{"tool": "mcp__sf__query", "action": "allow"}]),
                    "ask",
                ),
                Some(json!({ "sf": sf })),
            ),
            ("D2", "two", sandboxed(), Some(json!({ "sf": sf }))),
            // D3's `sf` differs: it becomes `sf-2`, and its rules follow.
            (
                "D3",
                "three",
                old_policy(
                    "custom",
                    json!([
                        {"tool": "mcp__sf__*", "action": "ask"},
                        {"tool": "mcp__sfx__run", "action": "deny"},
                        {"tool": "mcp__sf", "action": "deny"}
                    ]),
                    "ask",
                ),
                Some(json!({"sf": {"command": "sf-mcp", "args": ["--org", "other"]}})),
            ),
            // An http server without `type`, under a name that is not a valid one now, and its
            // Authorization header; a Trusted policy with the old `mcp__*` ask rule.
            (
                "D4",
                "four",
                old_policy(
                    "trusted",
                    json!([
                        {"tool": "Bash(git push:*)", "action": "ask"},
                        {"tool": "mcp__*", "action": "ask"},
                        {"tool": "mcp__GitHub_com__list", "action": "allow"},
                        {"tool": "mcp__GitHub.com__get", "action": "deny"}
                    ]),
                    "allow",
                ),
                Some(json!({
                    "GitHub.com": {"url": "https://api.example.com/mcp",
                                   "headers": {"Authorization": "Bearer xyz", "Accept": "a",
                                               "X-Api-Key": "k"}},
                    "broken": 3
                })),
            ),
            (
                "D5",
                "five",
                old_policy(
                    "read-only",
                    json!([{"tool": "Read", "action": "allow"}]),
                    "deny",
                ),
                None,
            ),
            // A custom policy may hold `mcp__*` ask on purpose: only Trusted's is the preset's.
            (
                "D6",
                "six",
                old_policy(
                    "custom",
                    json!([{"tool": "mcp__*", "action": "ask"}]),
                    "ask",
                ),
                Some(json!({})),
            ),
        ],
    )
    .await;

    let store = Store::open(&path).await.unwrap();
    let items = store.list_library_items().await.unwrap();
    let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["github_com", "sf", "sf-2"]);
    for item in &items {
        assert_eq!(item.owner_id, None);
        assert_eq!(
            store.get_catalog(&item.id).await.unwrap(),
            McpCatalog::unchecked(&item.id)
        );
    }
    let config = |name: &str| -> Value {
        let item = items.iter().find(|i| i.name == name).unwrap();
        serde_json::to_value(&item.config).unwrap()
    };
    // A secret-looking value becomes a variable; others, and references, are copied.
    assert_eq!(
        config("sf"),
        json!({"kind": "mcp_server", "config": {
            "transport": "stdio", "command": "sf-mcp", "args": ["--org", "acme"], "cwd": null,
            "env": {"API_KEY": "${MY_KEY}", "MODE": "readonly", "SF_TOKEN": "${SF_TOKEN}"}}})
    );
    assert_eq!(config("sf-2")["config"]["args"], json!(["--org", "other"]));
    assert_eq!(
        config("github_com"),
        json!({"kind": "mcp_server", "config": {
            "transport": "http", "url": "https://api.example.com/mcp",
            "headers": {"Accept": "a", "Authorization": "Bearer ${GITHUB_COM_TOKEN}",
                        "X-Api-Key": "${X_API_KEY}"}}})
    );

    let dot = |id: &str| {
        let store = store.clone();
        let id = id.to_string();
        async move { store.get_dot(&id).await.unwrap().spec }
    };
    let d1 = dot("D1").await;
    assert_eq!(d1.uses, uses(&["sf"], &[], &[]));
    assert_eq!(
        d1.policy.rules,
        [
            Rule::new("mcp__sf__query", Action::Allow),
            Rule::new("Skill", Action::Allow),
            Rule::new("Context", Action::Allow),
        ]
    );
    assert_eq!(d1.policy.mcp_default, Action::Ask);
    assert_eq!(dot("D2").await.uses, uses(&["sf"], &[], &[]));
    let d3 = dot("D3").await;
    assert_eq!(d3.uses, uses(&["sf-2"], &[], &[]));
    assert_eq!(
        d3.policy.rules,
        [
            Rule::new("mcp__sf-2__*", Action::Ask),
            Rule::new("mcp__sfx__run", Action::Deny),
            Rule::new("mcp__sf-2", Action::Deny),
            Rule::new("Skill", Action::Allow),
            Rule::new("Context", Action::Allow),
        ]
    );
    let d4 = dot("D4").await;
    assert_eq!(d4.uses, uses(&["github_com"], &[], &[]));
    assert_eq!(d4.policy.preset, Preset::Trusted);
    assert_eq!(
        d4.policy.rules,
        [
            Rule::new("Bash(git push:*)", Action::Ask),
            Rule::new("mcp__github_com__list", Action::Allow),
            Rule::new("mcp__github_com__get", Action::Deny),
            Rule::new("Skill", Action::Allow),
            Rule::new("Context", Action::Allow),
            // D4 is Trusted: migration 0008 adds the check-in tools, and 0012 `Memory`.
            Rule::new("Notify", Action::Allow),
            Rule::new("Ask", Action::Allow),
            Rule::new("FollowUp", Action::Allow),
            Rule::new("Memory", Action::Allow),
        ]
    );
    assert_eq!(d4.policy.mcp_default, Action::Ask);
    let d5 = dot("D5").await;
    assert_eq!(d5.uses, Uses::default());
    assert_eq!(d5.policy.mcp_default, Action::Deny);
    let d6 = dot("D6").await;
    assert_eq!(
        d6.policy.rules,
        [
            Rule::new("mcp__*", Action::Ask),
            Rule::new("Skill", Action::Allow),
            Rule::new("Context", Action::Allow),
        ]
    );
    assert_eq!(d6.uses, Uses::default());
    store.close().await;

    // The policy is written with `mcp_default`; the old column and the waiting rows are gone;
    // approvals have the fingerprint column.
    let pool = SqlitePool::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    let raw: String = sqlx::query_scalar("SELECT policy FROM dots WHERE id = 'D5'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let stored: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(stored["mcp_default"], "deny");
    let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info('dots')")
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(columns.contains(&"uses".to_string()));
    assert!(!columns.contains(&"mcp_servers".to_string()));
    let approvals: Vec<String> =
        sqlx::query_scalar("SELECT name FROM pragma_table_info('approvals')")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert!(approvals.contains(&"mcp_fingerprint".to_string()));
    let waiting: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'upgrade_0006'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(waiting, 0);
    pool.close().await;

    // Opening again converts nothing twice.
    let store = Store::open(&path).await.unwrap();
    assert_eq!(store.list_library_items().await.unwrap().len(), 3);
    assert_eq!(
        store.get_dot("D6").await.unwrap().spec.policy.rules,
        [
            Rule::new("mcp__*", Action::Ask),
            Rule::new("Skill", Action::Allow),
            Rule::new("Context", Action::Allow),
        ]
    );
}

/// Equal names with different inline credentials are different items, and no secret value is
/// left in the database file.
#[tokio::test]
async fn migration_0006_keeps_credentials_apart_and_erases_them_from_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let policy = || {
        old_policy(
            "sandboxed",
            json!([{"tool": "mcp__gh__*", "action": "deny"}]),
            "ask",
        )
    };
    let gh = |token: &str| {
        json!({"gh": {"url": "https://gh.example.com/mcp",
                      "headers": {"Authorization": format!("Bearer {token}")}}})
    };
    let sf = |token: &str| json!({"sf": {"command": "sf-mcp", "env": {"SF_TOKEN": token}}});
    let mut both_a = gh("tokA-Zq81xyzAAA");
    both_a
        .as_object_mut()
        .unwrap()
        .extend(sf("sfA-Zq81xyzAAA").as_object().unwrap().clone());
    let mut both_b = gh("tokB-Zq81xyzBBB");
    both_b
        .as_object_mut()
        .unwrap()
        .extend(sf("sfB-Zq81xyzBBB").as_object().unwrap().clone());
    database_0005(
        &path,
        &[
            ("DA", "a", policy(), Some(both_a)),
            ("DB", "b", policy(), Some(both_b)),
        ],
    )
    .await;
    let store = Store::open(&path).await.unwrap();
    let names: Vec<String> = store
        .list_library_items()
        .await
        .unwrap()
        .into_iter()
        .map(|i| i.name)
        .collect();
    assert_eq!(names, ["gh", "gh-2", "sf", "sf-2"]);
    let a = store.get_dot("DA").await.unwrap().spec;
    let b = store.get_dot("DB").await.unwrap().spec;
    assert_eq!(a.uses, uses(&["gh", "sf"], &[], &[]));
    assert_eq!(b.uses, uses(&["gh-2", "sf-2"], &[], &[]));
    // Migration 0007 adds `Context` after the `Skill` that 0006 adds, and 0008 the check-in
    // tools after it, and 0012 `Memory`: both dots are Sandboxed.
    let tail = [
        Rule::new("Skill", Action::Allow),
        Rule::new("Context", Action::Allow),
        Rule::new("Notify", Action::Allow),
        Rule::new("Ask", Action::Allow),
        Rule::new("FollowUp", Action::Allow),
        Rule::new("Memory", Action::Allow),
    ];
    let with_tail = |first: Rule| [vec![first], tail.to_vec()].concat();
    assert_eq!(
        a.policy.rules,
        with_tail(Rule::new("mcp__gh__*", Action::Deny))
    );
    assert_eq!(
        b.policy.rules,
        with_tail(Rule::new("mcp__gh-2__*", Action::Deny))
    );
    store.close().await;

    for file in [path.clone(), dir.path().join("dots.db-wal")] {
        let Ok(bytes) = std::fs::read(&file) else {
            continue;
        };
        let secret = "Zq81xyz";
        assert!(
            !bytes.windows(secret.len()).any(|w| w == secret.as_bytes()),
            "{} still holds {secret}",
            file.display()
        );
    }
}

/// A policy stored with `mcp_default` (written by a build that already had it) keeps its own.
#[tokio::test]
async fn migration_0006_keeps_a_stored_mcp_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let mut policy = Policy::preset(Preset::ReadOnly);
    policy.mcp_default = Action::Allow;
    database_0005(
        &path,
        &[("D1", "one", serde_json::to_value(&policy).unwrap(), None)],
    )
    .await;
    let store = Store::open(&path).await.unwrap();
    assert_eq!(store.get_dot("D1").await.unwrap().spec.policy, policy);
}

/// Every stored policy gets the `Skill` allow rule every preset has, unless a rule of its own
/// already names `Skill`.
#[tokio::test]
async fn migration_0006_allows_skill_unless_a_rule_names_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    database_0005(
        &path,
        &[
            ("D1", "one", old_policy("trusted", json!([]), "allow"), None),
            (
                "D2",
                "two",
                old_policy(
                    "custom",
                    json!([{"tool": "Skill", "action": "deny"}]),
                    "ask",
                ),
                None,
            ),
            (
                "D3",
                "three",
                old_policy(
                    "custom",
                    json!([{"tool": "Skill(pdf)", "action": "ask"}]),
                    "ask",
                ),
                None,
            ),
        ],
    )
    .await;
    let store = Store::open(&path).await.unwrap();
    let rules = |id: &'static str| {
        let store = store.clone();
        async move { store.get_dot(id).await.unwrap().spec.policy.rules }
    };
    let context = Rule::new("Context", Action::Allow);
    // D1 is Trusted: migration 0008 adds the check-in tools, and 0012 `Memory`. D2 and D3 are
    // Custom.
    assert_eq!(
        rules("D1").await,
        [
            Rule::new("Skill", Action::Allow),
            context.clone(),
            Rule::new("Notify", Action::Allow),
            Rule::new("Ask", Action::Allow),
            Rule::new("FollowUp", Action::Allow),
            Rule::new("Memory", Action::Allow),
        ]
    );
    assert_eq!(
        rules("D2").await,
        [Rule::new("Skill", Action::Deny), context.clone()]
    );
    assert_eq!(
        rules("D3").await,
        [Rule::new("Skill(pdf)", Action::Ask), context]
    );
}

/// A stored policy whose `default` is `deny` denies MCP tools too, whatever its preset.
#[tokio::test]
async fn migration_0006_gives_a_policy_that_denies_by_default_a_deny_mcp_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    database_0005(
        &path,
        &[
            ("D1", "one", old_policy("custom", json!([]), "deny"), None),
            (
                "D2",
                "two",
                old_policy("sandboxed", json!([]), "deny"),
                None,
            ),
            ("D3", "three", old_policy("custom", json!([]), "ask"), None),
            (
                "D4",
                "four",
                old_policy("trusted", json!([]), "allow"),
                None,
            ),
        ],
    )
    .await;
    let store = Store::open(&path).await.unwrap();
    for (id, want) in [
        ("D1", Action::Deny),
        ("D2", Action::Deny),
        ("D3", Action::Ask),
        ("D4", Action::Ask),
    ] {
        let policy = store.get_dot(id).await.unwrap().spec.policy;
        assert_eq!(policy.mcp_default, want, "{id}");
    }
}

/// An inline `Authorization` value keeps its scheme word; only the credential becomes the
/// variable. A value without a scheme word, or with a first word that is not one, gets `Bearer`.
#[tokio::test]
async fn migration_0006_keeps_the_scheme_of_an_authorization_header() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let server = |auth: &str| json!({"url": "https://api.example.com/mcp", "headers": {"Authorization": auth}});
    database_0005(
        &path,
        &[(
            "D1",
            "one",
            old_policy("custom", json!([]), "ask"),
            Some(json!({
                "basic": server("Basic dXNlcjpwYXNz"),
                "token": server("token ghp_Zq81xyz"),
                "bearer": server("Bearer Zq81xyz"),
                "bare": server("Zq81xyz"),
                "odd": server("Zq81/xyz= more"),
            })),
        )],
    )
    .await;
    let store = Store::open(&path).await.unwrap();
    let items = store.list_library_items().await.unwrap();
    let auth = |name: &str| -> String {
        let item = items.iter().find(|i| i.name == name).unwrap();
        let LibraryConfig::McpServer(McpServerConfig::Http { headers, .. }) = &item.config else {
            panic!("{:?}", item.config)
        };
        headers["Authorization"].clone()
    };
    assert_eq!(auth("basic"), "Basic ${BASIC_TOKEN}");
    assert_eq!(auth("token"), "token ${TOKEN_TOKEN}");
    assert_eq!(auth("bearer"), "Bearer ${BEARER_TOKEN}");
    assert_eq!(auth("bare"), "Bearer ${BARE_TOKEN}");
    assert_eq!(auth("odd"), "Bearer ${ODD_TOKEN}");
}

fn http(url: &str, header: &str) -> LibraryConfig {
    LibraryConfig::McpServer(McpServerConfig::Http {
        url: url.into(),
        headers: BTreeMap::from([("X-Key".to_string(), header.to_string())]),
    })
}

#[tokio::test]
async fn moving_an_http_server_to_another_origin_drops_what_was_accepted() {
    let (_d, store) = temp_store().await;
    let tools = json!([{"name": "t"}]);
    let item = store
        .add_library_item("web", &http("https://a.example/mcp", "1"), None)
        .await
        .unwrap();
    let accept = || async {
        store
            .record_catalog_seen(&item.id, "f1", &tools)
            .await
            .unwrap();
        store.accept_catalog(&item.id, "f1").await.unwrap();
    };
    accept().await;

    // Same origin (another path and header): the accepted list stays for the review.
    store
        .update_library_item(&item.id, &http("https://A.example/other", "2"))
        .await
        .unwrap();
    let c = store.get_catalog(&item.id).await.unwrap();
    assert_eq!(c.status, CatalogStatus::Unchecked);
    assert_eq!(c.accepted_fingerprint.as_deref(), Some("f1"));
    let c = store
        .record_catalog_seen(&item.id, "f1", &tools)
        .await
        .unwrap();
    assert_eq!(c.status, CatalogStatus::Accepted);

    // Another host with the same tool list is not accepted again silently.
    store
        .update_library_item(&item.id, &http("https://b.example/mcp", "2"))
        .await
        .unwrap();
    let c = store.get_catalog(&item.id).await.unwrap();
    assert_eq!(c.accepted_fingerprint, None);
    assert_eq!(c.accepted_tools, None);
    let c = store
        .record_catalog_seen(&item.id, "f1", &tools)
        .await
        .unwrap();
    assert_eq!(c.status, CatalogStatus::Changed);
}

#[tokio::test]
async fn a_url_moves_the_origin_to_the_host_the_client_reaches_after_expanding() {
    let (_d, store) = temp_store().await;
    let tools = json!([{"name": "t"}]);
    let item = store
        .add_library_item("web", &http("https://api.github.com/mcp", "1"), None)
        .await
        .unwrap();
    store
        .record_catalog_seen(&item.id, "f1", &tools)
        .await
        .unwrap();
    store.accept_catalog(&item.id, "f1").await.unwrap();
    // The default port is the same origin: the accepted list stays.
    store
        .update_library_item(&item.id, &http("https://api.github.com:443/mcp", "1"))
        .await
        .unwrap();
    let c = store.get_catalog(&item.id).await.unwrap();
    assert_eq!(c.accepted_fingerprint.as_deref(), Some("f1"));
    // A variable that holds a backslash makes this url reach evil.example, though the raw text
    // reads like the trusted host.
    std::env::set_var("DOTS_TEST_BACKSLASH", "\\");
    store
        .update_library_item(
            &item.id,
            &http(
                "https://evil.example${DOTS_TEST_BACKSLASH}@api.github.com/mcp",
                "1",
            ),
        )
        .await
        .unwrap();
    let c = store.get_catalog(&item.id).await.unwrap();
    assert_eq!(c.accepted_fingerprint, None);
    // An unparsable url (a variable in the host) is a move unless the text is equal.
    let var = |p: &str| http(&format!("https://${{HOST}}/{p}"), "1");
    store
        .update_library_item(&item.id, &var("a"))
        .await
        .unwrap();
    store
        .record_catalog_seen(&item.id, "f1", &tools)
        .await
        .unwrap();
    store.accept_catalog(&item.id, "f1").await.unwrap();
    store
        .update_library_item(&item.id, &var("a"))
        .await
        .unwrap();
    let c = store.get_catalog(&item.id).await.unwrap();
    assert_eq!(c.accepted_fingerprint.as_deref(), Some("f1"));
    store
        .update_library_item(&item.id, &var("b"))
        .await
        .unwrap();
    let c = store.get_catalog(&item.id).await.unwrap();
    assert_eq!(c.accepted_fingerprint, None);
}
