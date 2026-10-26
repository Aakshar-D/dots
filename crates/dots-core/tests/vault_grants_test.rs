//! What a dot is granted and how its variables resolve (env vault spec §7, §8): a server the
//! dot ticks is granted the variables of its config, a name resolves only when it is granted
//! and only from Personal, and what does not resolve leaves the server out with the reason.

mod common;

use std::collections::BTreeMap;

use common::fake_mcp::{tool, FakeMcp};
use common::{spec, temp_store};
use dots_core::events::new_bus;
use dots_core::library::{LibraryConfig, McpServerConfig, PluginConfig, Uses, PERSONAL_SCOPE};
use dots_core::run_tools::{OpenRequest, RunTools};
use dots_core::store::Store;
use dots_core::vault::{Consumer, DotScope, EntryKind, Grants, Missing, Unresolved, Vault};
use dots_core::Error;
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use tokio_util::sync::CancellationToken;

fn grants(pairs: &[(&str, &[&str])]) -> Grants {
    Grants(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.iter().map(|s| s.to_string()).collect()))
            .collect(),
    )
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn server_with(env: &[(&str, &str)]) -> LibraryConfig {
    LibraryConfig::McpServer(McpServerConfig::Stdio {
        command: "node".into(),
        args: Vec::new(),
        cwd: None,
        env: env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    })
}

#[test]
fn a_consumer_is_a_server_or_the_shell() {
    assert_eq!(Consumer::Mcp("github".into()).key(), "mcp:github");
    assert_eq!(Consumer::Shell.key(), "shell");
    assert_eq!(
        Consumer::parse("mcp:github").unwrap(),
        Consumer::Mcp("github".into())
    );
    assert_eq!(Consumer::parse("shell").unwrap(), Consumer::Shell);
    for bad in ["github", "mcp:", "mcp:Git Hub", "mcp:dots", "Shell"] {
        assert!(
            matches!(Consumer::parse(bad), Err(Error::Invalid(_))),
            "{bad}"
        );
    }
}

#[test]
fn grants_name_consumers_and_variables_once() {
    grants(&[("mcp:github", &["GITHUB_TOKEN"]), ("shell", &["LOG_LEVEL"])])
        .validate()
        .unwrap();
    for bad in [
        grants(&[("github", &["A"])]),
        grants(&[("shell", &["1A"])]),
        grants(&[("shell", &["A", "a"])]),
    ] {
        assert!(matches!(bad.validate(), Err(Error::Invalid(_))), "{bad:?}");
    }
    let g = grants(&[("mcp:github", &["GITHUB_TOKEN"])]);
    assert!(g.allows(&Consumer::Mcp("github".into()), "github_token"));
    assert!(!g.allows(&Consumer::Mcp("other".into()), "GITHUB_TOKEN"));
    assert!(!g.allows(&Consumer::Shell, "GITHUB_TOKEN"));
}

#[test]
fn a_save_grants_a_newly_used_server_its_variables_and_drops_an_unused_one() {
    let given = grants(&[
        ("shell", &["LOG_LEVEL"]),
        ("mcp:kept", &["A"]),
        ("mcp:gone", &["B"]),
    ]);
    let using = vec![
        ("kept".to_string(), names(&["A", "NEW_IN_CONFIG"])),
        ("fresh".to_string(), names(&["C", "D"])),
    ];
    let synced = given.synced(&names(&["kept", "gone"]), &using);
    assert_eq!(
        synced,
        grants(&[
            ("shell", &["LOG_LEVEL"]),
            // Still used: kept as given. A variable its config gained is not granted.
            ("mcp:kept", &["A"]),
            // Newly used: granted what its config reads.
            ("mcp:fresh", &["C", "D"]),
        ])
    );
}

#[tokio::test]
async fn ticking_a_server_grants_its_variables_and_a_later_one_waits() {
    let (_dir, store) = temp_store().await;
    let gh = store
        .add_library_item(
            "github",
            &server_with(&[("TOKEN", "${GITHUB_TOKEN}")]),
            None,
        )
        .await
        .unwrap();
    store
        .add_library_item("plain", &server_with(&[("MODE", "readonly")]), None)
        .await
        .unwrap();
    let mut s = spec("triage");
    s.uses.mcp_servers = names(&["github", "plain"]);
    let dot = store.create_dot(&s, None).await.unwrap();
    assert_eq!(
        dot.spec.variables,
        grants(&[("mcp:github", &["GITHUB_TOKEN"])])
    );

    // The server's config gains a variable: the dot is not granted it.
    store
        .update_library_item(
            &gh.id,
            &server_with(&[("TOKEN", "${GITHUB_TOKEN}"), ("ORG", "${GITHUB_ORG}")]),
        )
        .await
        .unwrap();
    let saved = store.update_dot(&dot.id, &dot.spec).await.unwrap();
    assert_eq!(
        saved.spec.variables,
        grants(&[("mcp:github", &["GITHUB_TOKEN"])])
    );

    // Granting it by hand keeps it; unticking the server drops its grants.
    let mut granted = saved.spec.clone();
    granted.variables = grants(&[
        ("mcp:github", &["GITHUB_TOKEN", "GITHUB_ORG"]),
        ("shell", &["LOG_LEVEL"]),
    ]);
    let saved = store.update_dot(&dot.id, &granted).await.unwrap();
    assert_eq!(saved.spec.variables, granted.variables);
    let mut unticked = saved.spec.clone();
    unticked.uses.mcp_servers = names(&["plain"]);
    let saved = store.update_dot(&dot.id, &unticked).await.unwrap();
    assert_eq!(saved.spec.variables, grants(&[("shell", &["LOG_LEVEL"])]));

    // A grant for something that is not a consumer is refused.
    let mut bad = saved.spec.clone();
    bad.variables = grants(&[("github", &["GITHUB_TOKEN"])]);
    assert!(matches!(
        store.update_dot(&dot.id, &bad).await,
        Err(Error::Invalid(_))
    ));
}

#[tokio::test]
async fn ticking_a_plugin_grants_the_variables_of_its_servers() {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let root = dir.path().join("forms");
    std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
    std::fs::write(
        root.join(".claude-plugin").join("plugin.json"),
        r#"{"name":"forms"}"#,
    )
    .unwrap();
    std::fs::write(
        root.join(".mcp.json"),
        json!({"mcpServers": {"db": {"command": "node", "env": {"T": "${DB_TOKEN}"}}}}).to_string(),
    )
    .unwrap();
    let lib = dots_core::library_service::LibraryService::new(store.clone(), bus);
    lib.add(
        "forms",
        LibraryConfig::Plugin(PluginConfig {
            path: root.to_string_lossy().into_owned(),
            installed_id: None,
            run_hooks: false,
        }),
        None,
    )
    .await
    .unwrap();
    let mut s = spec("filler");
    s.uses.plugins = names(&["forms"]);
    let dot = store.create_dot(&s, None).await.unwrap();
    assert_eq!(
        dot.spec.variables,
        grants(&[("mcp:plugin-forms-db", &["DB_TOKEN"])])
    );
}

/// A plugin update that adds a `${VAR}` to a server of a plugin the dot already ticks does not
/// grant it (env vault spec §13): the dot is asked first.
#[tokio::test]
async fn a_plugin_update_adding_a_variable_does_not_grant_it() {
    let (dir, store) = temp_store().await;
    let root = dir.path().join("forms");
    std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
    std::fs::write(
        root.join(".claude-plugin").join("plugin.json"),
        r#"{"name":"forms"}"#,
    )
    .unwrap();
    let mcp = |env: Value| json!({"mcpServers": {"db": {"command": "node", "env": env}}});
    std::fs::write(
        root.join(".mcp.json"),
        mcp(json!({"T": "${DB_TOKEN}"})).to_string(),
    )
    .unwrap();
    let lib = dots_core::library_service::LibraryService::new(store.clone(), new_bus());
    let plugin = LibraryConfig::Plugin(PluginConfig {
        path: root.to_string_lossy().into_owned(),
        installed_id: None,
        run_hooks: false,
    });
    let item = lib.add("forms", plugin.clone(), None).await.unwrap();
    let mut s = spec("filler");
    s.uses.plugins = names(&["forms"]);
    let dot = store.create_dot(&s, None).await.unwrap();
    // The plugin's update reads one more variable, and the plugin is synced again.
    std::fs::write(
        root.join(".mcp.json"),
        mcp(json!({"T": "${DB_TOKEN}", "U": "${DB_ADMIN_TOKEN}"})).to_string(),
    )
    .unwrap();
    lib.update(&item.id, plugin).await.unwrap();
    let server = store
        .find_library_item(
            dots_core::library::LibraryKind::McpServer,
            "plugin-forms-db",
        )
        .await
        .unwrap()
        .unwrap();
    let LibraryConfig::McpServer(config) = &server.config else {
        panic!("not a server");
    };
    assert_eq!(config.variables(), ["DB_ADMIN_TOKEN", "DB_TOKEN"]);
    let want = grants(&[("mcp:plugin-forms-db", &["DB_TOKEN"])]);
    assert_eq!(store.get_dot(&dot.id).await.unwrap().spec.variables, want);
    // Nor does saving the dot again.
    let saved = store.update_dot(&dot.id, &dot.spec).await.unwrap();
    assert_eq!(saved.spec.variables, want);
}

#[tokio::test]
async fn a_name_resolves_only_when_granted_and_only_from_personal() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store);
    vault
        .add("GITHUB_TOKEN", EntryKind::Secret, "ghp_value")
        .await
        .unwrap();
    std::env::set_var("DOTS_VAULT_ONLY_IN_ENV", "from-the-environment");
    let github = Consumer::Mcp("github".into());
    let g = grants(&[("mcp:github", &["github_token", "DOTS_VAULT_ONLY_IN_ENV"])]);

    let ok = vault
        .resolve(&DotScope::Personal, &g, &github, &names(&["GITHUB_TOKEN"]))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ok.get("GITHUB_TOKEN").as_deref(), Some("ghp_value"));

    let missing = vault
        .resolve(
            &DotScope::Personal,
            &g,
            &github,
            &names(&["GITHUB_TOKEN", "DOTS_VAULT_ONLY_IN_ENV"]),
        )
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(missing.why, Unresolved::NotDefined);
    assert_eq!(
        missing.text("triage"),
        "DOTS_VAULT_ONLY_IN_ENV: not in Personal (dot 'triage', mcp:github)"
    );

    let refused: Missing = vault
        .resolve(
            &DotScope::Personal,
            &g,
            &Consumer::Shell,
            &names(&["GITHUB_TOKEN"]),
        )
        .await
        .unwrap()
        .unwrap_err();
    assert_eq!(
        refused.text("triage"),
        "GITHUB_TOKEN: not granted to shell in dot 'triage'"
    );
}

async fn open(
    store: &Store,
    vault: &Vault,
    uses: &Uses,
    grants: &Grants,
    dir: &std::path::Path,
) -> (RunTools, Vec<String>) {
    RunTools::open(OpenRequest {
        store,
        bus: &new_bus(),
        run_id: "01RUN",
        uses,
        workspace: dir,
        plugins_dir: None,
        cancel: &CancellationToken::new(),
        vault,
        scope: &DotScope::Personal,
        grants,
        dot_name: "triage",
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn a_run_starts_a_server_with_its_granted_values_and_leaves_out_one_without() {
    let (dir, store) = temp_store().await;
    let vault = Vault::in_memory(store.clone());
    vault
        .add("GH_TOKEN", EntryKind::Secret, "ghp_run")
        .await
        .unwrap();
    vault
        .add("LOG_LEVEL", EntryKind::Plain, "debug")
        .await
        .unwrap();
    let fake = FakeMcp::new(json!({"tools": [tool("search", true)]}));
    let config = fake.stdio_with(
        Vec::new(),
        BTreeMap::from([("FAKE_MCP_TOKEN".to_string(), "${GH_TOKEN}".to_string())]),
    );
    let item = store
        .add_library_item("gh", &LibraryConfig::McpServer(config), None)
        .await
        .unwrap();
    let uses = Uses {
        mcp_servers: names(&["gh"]),
        ..Default::default()
    };

    // Not granted: left out, said why, and the Library's catalog is not marked.
    let none = grants(&[("shell", &["LOG_LEVEL", "NOT_THERE"])]);
    let (tools, notices) = open(&store, &vault, &uses, &none, dir.path()).await;
    assert!(tools.servers().is_empty());
    assert!(fake.log().is_empty());
    assert!(notices.contains(&"GH_TOKEN: not granted to mcp:gh in dot 'triage'".to_string()));
    assert!(notices.contains(&"NOT_THERE: not in Personal (dot 'triage', shell)".to_string()));
    let shell: Vec<(&str, &str)> = tools
        .shell_env()
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    assert_eq!(shell, [("LOG_LEVEL", "debug")]);
    let catalog = store.get_catalog(&item.id, PERSONAL_SCOPE).await.unwrap();
    assert_eq!(catalog.error, None);
    tools.close().await;

    // Granted: the server starts with the value.
    let granted = grants(&[("mcp:gh", &["GH_TOKEN"])]);
    let (tools, notices) = open(&store, &vault, &uses, &granted, dir.path()).await;
    assert_eq!(tools.servers().len(), 1, "{notices:?}");
    let start = fake
        .log()
        .into_iter()
        .find_map(|r| r.get("start").cloned())
        .unwrap();
    assert_eq!(start["env"]["FAKE_MCP_TOKEN"], "ghp_run");
    tools.close().await;
}

/// Migration 0011 on a database Dots left before the vault: each dot is granted the variables
/// of the servers it uses.
#[tokio::test]
async fn migration_0011_grants_each_dot_the_variables_of_its_servers() {
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
        .filter(|m| m.version < 11)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO library_items (id, kind, name, config, owner_id, created_at, updated_at) \
         VALUES ('S1', 'mcp_server', 'github', ?, NULL, '2026-10-01T00:00:00.000Z', \
         '2026-10-01T00:00:00.000Z')",
    )
    .bind(
        json!({"transport": "http", "url": "https://api.github.com/mcp",
               "headers": {"Authorization": "Bearer ${GITHUB_TOKEN}"}})
        .to_string(),
    )
    .execute(&pool)
    .await
    .unwrap();
    let policy = serde_json::to_string(&dots_core::policy::Policy::preset(
        dots_core::policy::Preset::Sandboxed,
    ))
    .unwrap();
    sqlx::query(
        "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
         webhook_token, policy, uses, max_turns, timeout_secs, approval_wait_secs, \
         created_at, updated_at) VALUES ('D1', 'triage', 'Do it.', 'claude', 'sonnet', \
         'C:/tmp/repo', 'worktree', 't', ?, ?, 40, 1800, 240, '2026-10-01T00:00:00.000Z', \
         '2026-10-01T00:00:00.000Z')",
    )
    .bind(&policy)
    .bind(json!({"mcp_servers": ["github"]}).to_string())
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;

    let store = Store::open(&path).await.unwrap();
    let want = grants(&[("mcp:github", &["GITHUB_TOKEN"])]);
    let dot = store.get_dot("D1").await.unwrap();
    assert_eq!(dot.spec.variables, want);
    // Its webhook token is kept as a hash, and still works.
    assert!(dot.has_webhook_token);
    assert!(store.check_webhook_token("D1", "t").await.unwrap());
    store.close().await;
    let store = Store::open(&path).await.unwrap();
    assert_eq!(store.get_dot("D1").await.unwrap().spec.variables, want);
}

#[tokio::test]
async fn a_name_personal_cannot_hold_is_not_auto_granted_and_the_dot_still_saves() {
    let (_dir, store) = temp_store().await;
    let long = "A".repeat(129);
    store
        .add_library_item(
            "wide",
            &server_with(&[("LONG", &format!("${{{long}}}")), ("OK", "${GOOD}")]),
            None,
        )
        .await
        .unwrap();
    let mut s = spec("wide-dot");
    s.uses.mcp_servers = names(&["wide"]);
    let dot = store.create_dot(&s, None).await.unwrap();
    assert_eq!(dot.spec.variables, grants(&[("mcp:wide", &["GOOD"])]));
    store.update_dot(&dot.id, &dot.spec).await.unwrap();
}

#[tokio::test]
async fn resolved_values_never_print() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store);
    vault
        .add("DEMO_TOKEN", EntryKind::Secret, "s3cret-made-up-value")
        .await
        .unwrap();
    let github = Consumer::Mcp("github".into());
    let g = grants(&[("mcp:github", &["DEMO_TOKEN"])]);
    let resolved = vault
        .resolve(&DotScope::Personal, &g, &github, &names(&["DEMO_TOKEN"]))
        .await
        .unwrap()
        .unwrap();
    let shown = format!("{resolved:?}");
    assert!(shown.contains("DEMO_TOKEN"), "{shown}");
    assert!(!shown.contains("s3cret-made-up-value"), "{shown}");
    // What does not resolve holds names and a consumer only.
    let missing = vault
        .resolve(&DotScope::Personal, &g, &github, &names(&["OTHER"]))
        .await
        .unwrap()
        .unwrap_err();
    assert!(!format!("{missing:?}").contains("s3cret-made-up-value"));
}

/// A database left before the vault (0011) with one dot, `D1`, whose webhook token column
/// holds `token`.
async fn db_before_0011_with_token(path: &std::path::Path, token: &str) {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version < 11)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    let policy = serde_json::to_string(&dots_core::policy::Policy::preset(
        dots_core::policy::Preset::Sandboxed,
    ))
    .unwrap();
    sqlx::query(
        "INSERT INTO dots (id, name, instructions, engine, model, workdir, workspace_mode, \
         webhook_token, policy, uses, max_turns, timeout_secs, approval_wait_secs, \
         created_at, updated_at) VALUES ('D1', 'triage', 'Do it.', 'claude', 'sonnet', \
         'C:/tmp/repo', 'worktree', ?, ?, '{}', 40, 1800, 240, '2026-10-01T00:00:00.000Z', \
         '2026-10-01T00:00:00.000Z')",
    )
    .bind(token)
    .bind(&policy)
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
}

/// A webhook token stored in clear is hashed, and the start-up vacuum leaves no copy of it in
/// the database file or its WAL, though no server has a stored secret value (env vault spec
/// §11). The old token still works.
#[tokio::test]
async fn a_start_with_old_webhook_tokens_vacuums_their_clear_values_away() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    // Made up, and distinctive enough to find in raw bytes.
    let token = "wHkTokenForTheVacuumTest0123456789ABCdefGH";
    db_before_0011_with_token(&path, token).await;

    assert!(
        std::fs::read(&path)
            .unwrap()
            .windows(token.len())
            .any(|w| w == token.as_bytes()),
        "the seed holds the token"
    );
    let store = Store::open(&path).await.unwrap();
    // Hashing a token leaves its old value in freed pages: the next vacuum is asked for.
    assert_eq!(
        store
            .marker(dots_core::vault::upgrade::VACUUM_PENDING)
            .await
            .unwrap(),
        Some(json!(true))
    );
    // What Runtime::start does next.
    let vault = Vault::new(
        store.clone(),
        std::sync::Arc::new(dots_core::vault::MemorySecrets::new()),
    );
    vault.move_stored_literals().await.unwrap();
    // The vacuum ran: the marker is cleared, not left for the next start.
    assert_eq!(
        store
            .marker(dots_core::vault::upgrade::VACUUM_PENDING)
            .await
            .unwrap(),
        Some(json!(false))
    );
    assert!(store.check_webhook_token("D1", token).await.unwrap());
    store.close().await;

    // The marker says the vacuum ran; the byte scan says it worked. It guards against a SQLite
    // build that keeps freed pages through a vacuum, and against a WAL left holding them.
    for name in ["dots.db", "dots.db-wal"] {
        let Ok(bytes) = std::fs::read(dir.path().join(name)) else {
            continue;
        };
        assert!(
            !bytes.windows(token.len()).any(|w| w == token.as_bytes()),
            "{name} still holds the old token"
        );
    }
}

/// A value that already is a hash is left as it is: it is not hashed twice.
#[tokio::test]
async fn a_token_that_is_already_a_hash_is_not_hashed_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    db_before_0011_with_token(&path, &dots_core::util::token_hash("kept")).await;
    let store = Store::open(&path).await.unwrap();
    assert!(store.check_webhook_token("D1", "kept").await.unwrap());
    // Nothing was hashed, so no vacuum is asked for.
    assert_eq!(
        store
            .marker(dots_core::vault::upgrade::VACUUM_PENDING)
            .await
            .unwrap(),
        None
    );
    store.close().await;
}

/// A database Dots left at 0012 at `path`, holding the Library `servers` (id, name, config)
/// and the plain Personal variables `plain`.
async fn db_at_0012(
    path: &std::path::Path,
    servers: &[(&str, &str, Value)],
    plain: &[(&str, &str)],
) {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let mut before = sqlx::migrate!("./migrations");
    before.migrations = before
        .migrations
        .iter()
        .filter(|m| m.version < 13)
        .cloned()
        .collect::<Vec<_>>()
        .into();
    before.run(&pool).await.unwrap();
    for (id, name, config) in servers {
        sqlx::query(
            "INSERT INTO library_items (id, kind, name, config, owner_id, created_at, \
             updated_at) VALUES (?, 'mcp_server', ?, ?, NULL, '2026-10-01T00:00:00.000Z', \
             '2026-10-01T00:00:00.000Z')",
        )
        .bind(id)
        .bind(name)
        .bind(config.to_string())
        .execute(&pool)
        .await
        .unwrap();
    }
    for (name, value) in plain {
        sqlx::query(
            "INSERT INTO env_entries (name, kind, value, created_at, updated_at) \
             VALUES (?, 'plain', ?, '2026-10-01T00:00:00.000Z', '2026-10-01T00:00:00.000Z')",
        )
        .bind(name)
        .bind(value)
        .execute(&pool)
        .await
        .unwrap();
    }
    pool.close().await;
}

/// Whether the database at `path` still has migration 0013's list of servers to do.
async fn upgrade_0013_waits(path: &std::path::Path) -> bool {
    let opts = SqliteConnectOptions::new().filename(path);
    let pool = SqlitePool::connect_with(opts).await.unwrap();
    let left: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'upgrade_0013'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    pool.close().await;
    left > 0
}

/// Migration 0013: each http server whose URL reads a variable gets the origin its URL reaches
/// with Personal's values now confirmed in Personal, so it keeps working; one whose origin
/// cannot be computed, a fixed URL (whatever its headers send) and a stdio server get none. It
/// runs once.
#[tokio::test]
async fn migration_0013_confirms_the_origin_each_http_server_reaches_now() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let http =
        |url: &str, headers: Value| json!({"transport": "http", "url": url, "headers": headers});
    db_at_0012(
        &path,
        &[
            (
                "S1",
                "hosted",
                http(
                    "https://${MCP_HOST}/mcp",
                    json!({"A": "Bearer ${GH_TOKEN}"}),
                ),
            ),
            (
                "S2",
                "unset",
                http("https://${NOT_SET}/mcp", json!({"A": "${GH_TOKEN}"})),
            ),
            (
                "S3",
                "plain",
                http("https://plain.example/mcp", json!({"A": "fixed"})),
            ),
            (
                "S4",
                "github",
                http(
                    "https://API.github.com:443/mcp",
                    json!({"A": "${GH_TOKEN}"}),
                ),
            ),
            (
                "S5",
                "local",
                json!({"transport": "stdio", "command": "node", "env": {"T": "${GH_TOKEN}"}}),
            ),
            (
                "S6",
                "github-path",
                http(
                    "https://API.github.com:443/${GH_PATH}",
                    json!({"A": "${GH_TOKEN}"}),
                ),
            ),
        ],
        &[("MCP_HOST", "mcp.example:8443"), ("GH_PATH", "mcp")],
    )
    .await;

    let store = Store::open(&path).await.unwrap();
    assert!(upgrade_0013_waits(&path).await);
    // What Runtime::start does once the vault is open.
    let vault = Vault::in_memory(store.clone());
    assert_eq!(store.finish_0013(&vault).await.unwrap(), 2);
    let confirmed = |pairs: &[&str]| -> BTreeMap<String, Vec<String>> {
        match pairs {
            [] => BTreeMap::new(),
            list => BTreeMap::from([(
                PERSONAL_SCOPE.to_string(),
                list.iter().map(|s| s.to_string()).collect(),
            )]),
        }
    };
    let want = [
        ("S1", confirmed(&["https://mcp.example:8443"])),
        ("S2", confirmed(&[])),
        ("S3", confirmed(&[])),
        ("S4", confirmed(&[])),
        ("S5", confirmed(&[])),
        ("S6", confirmed(&["https://api.github.com"])),
    ];
    for (id, origins) in &want {
        assert_eq!(&store.confirmed_origins(id).await.unwrap(), origins, "{id}");
    }
    assert!(!upgrade_0013_waits(&path).await, "the list is dropped");

    // Once only: values set or changed since are not recorded by a second start.
    vault
        .add("NOT_SET", EntryKind::Plain, "late.example")
        .await
        .unwrap();
    vault.set_value("MCP_HOST", "moved.example").await.unwrap();
    store.close().await;
    let store = Store::open(&path).await.unwrap();
    let vault = Vault::in_memory(store.clone());
    assert_eq!(store.finish_0013(&vault).await.unwrap(), 0);
    for (id, origins) in &want {
        assert_eq!(&store.confirmed_origins(id).await.unwrap(), origins, "{id}");
    }
}

/// A store with no Library items, as a new data folder has: nothing to record, and the list
/// is still dropped.
#[tokio::test]
async fn migration_0013_on_a_store_without_servers_records_nothing() {
    let (dir, store) = temp_store().await;
    let path = dir.path().join("dots.db");
    assert!(upgrade_0013_waits(&path).await);
    let vault = Vault::in_memory(store.clone());
    assert_eq!(store.finish_0013(&vault).await.unwrap(), 0);
    assert!(!upgrade_0013_waits(&path).await);
    assert_eq!(store.finish_0013(&vault).await.unwrap(), 0);
}

/// The runtime runs the step at its start: a server from before 0013 starts where it did.
#[tokio::test]
async fn a_runtime_start_confirms_the_origins_of_the_servers_from_before() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    db_at_0012(
        &data.join(dots_core::store::DB_FILE),
        &[(
            "S1",
            "hosted",
            json!({"transport": "http", "url": "https://${MCP_HOST}/mcp"}),
        )],
        &[("MCP_HOST", "mcp.example")],
    )
    .await;
    let mut cfg = dots_core::Config::new(data);
    cfg.port = 0;
    cfg.claude_path = Some(dir.path().join("no-claude.exe"));
    let rt = dots_core::Runtime::start(cfg).await.unwrap();
    assert_eq!(
        rt.store().confirmed_origins("S1").await.unwrap(),
        BTreeMap::from([(
            PERSONAL_SCOPE.to_string(),
            vec!["https://mcp.example".to_string()]
        )])
    );
    assert_eq!(rt.store().finish_0013(rt.vault()).await.unwrap(), 0);
    rt.shutdown();
}
