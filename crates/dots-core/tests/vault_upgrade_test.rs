//! Secret values typed into the Library move into Personal, and a setup from before the vault
//! is brought in (env vault spec §11).

mod common;

use std::path::Path;
use std::time::{Duration, SystemTime};

use common::{spec, temp_store};
use dots_core::events::new_bus;
use dots_core::library::{LibraryConfig, LibraryKind, McpServerConfig, PluginConfig};
use dots_core::library_service::LibraryService;
use dots_core::store::Store;
use dots_core::vault::{EntryKind, Vault};
use dots_core::Error;
use serde_json::json;

fn stdio(env: &[(&str, &str)]) -> McpServerConfig {
    McpServerConfig::Stdio {
        command: "node".into(),
        args: Vec::new(),
        cwd: None,
        env: env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

fn http(headers: &[(&str, &str)]) -> McpServerConfig {
    McpServerConfig::Http {
        url: "https://api.example.com/mcp".into(),
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    }
}

/// Whether the database file, with its WAL, holds `text`.
fn db_holds(dir: &Path, text: &str) -> bool {
    ["dots.db", "dots.db-wal"].iter().any(|f| {
        std::fs::read(dir.join(f))
            .map(|b| b.windows(text.len()).any(|w| w == text.as_bytes()))
            .unwrap_or(false)
    })
}

/// An old backup: a `.db` file in `<dir>/backups` an hour older than now.
fn old_backup(dir: &Path) -> std::path::PathBuf {
    let backups = dir.join("backups");
    std::fs::create_dir_all(&backups).unwrap();
    let old = backups.join("dots-0.1.0-2026-10-01-090000.db");
    std::fs::write(&old, b"old").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&old)
        .unwrap()
        .set_modified(SystemTime::now() - Duration::from_secs(3600))
        .unwrap();
    old
}

#[tokio::test]
async fn a_secret_looking_value_moves_into_personal_under_its_name() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store);
    let (config, moved) = vault
        .move_literals(
            "sf",
            &stdio(&[
                ("SF_TOKEN", "sf-literal-1"),
                ("MODE", "readonly"),
                ("API_KEY", "${MY_KEY}"),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(moved, ["SF_TOKEN"]);
    assert_eq!(
        config,
        stdio(&[
            ("SF_TOKEN", "${SF_TOKEN}"),
            ("MODE", "readonly"),
            ("API_KEY", "${MY_KEY}")
        ])
    );
    let (config, moved) = vault
        .move_literals(
            "git-hub",
            &http(&[
                ("Authorization", "token ghp_literal_2"),
                ("X-Api-Key", "key-literal-3"),
                ("Accept", "application/json"),
            ]),
        )
        .await
        .unwrap();
    assert_eq!(moved, ["GIT_HUB_AUTHORIZATION", "GIT_HUB_X_API_KEY"]);
    assert_eq!(
        config,
        http(&[
            ("Authorization", "token ${GIT_HUB_AUTHORIZATION}"),
            ("X-Api-Key", "${GIT_HUB_X_API_KEY}"),
            ("Accept", "application/json"),
        ])
    );
    let values = vault
        .personal_values(&["SF_TOKEN".into(), "GIT_HUB_AUTHORIZATION".into()])
        .await
        .unwrap();
    assert_eq!(values.get("SF_TOKEN").as_deref(), Some("sf-literal-1"));
    assert_eq!(
        values.get("GIT_HUB_AUTHORIZATION").as_deref(),
        Some("ghp_literal_2")
    );
    let listed = vault.list().await.unwrap();
    assert!(listed
        .iter()
        .all(|e| e.kind == EntryKind::Secret && e.value.is_none()));
}

#[tokio::test]
async fn the_same_value_reuses_its_name_and_another_value_gets_a_new_one() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store);
    vault
        .add("SF_TOKEN", EntryKind::Secret, "first")
        .await
        .unwrap();
    let (_, same) = vault
        .move_literals("a", &stdio(&[("SF_TOKEN", "first")]))
        .await
        .unwrap();
    assert_eq!(same, ["SF_TOKEN"]);
    let (config, other) = vault
        .move_literals("b", &stdio(&[("SF_TOKEN", "second")]))
        .await
        .unwrap();
    assert_eq!(other, ["SF_TOKEN_2"]);
    assert_eq!(config, stdio(&[("SF_TOKEN", "${SF_TOKEN_2}")]));
}

#[tokio::test]
async fn the_library_never_stores_a_typed_secret() {
    let (dir, store) = temp_store().await;
    let vault = Vault::in_memory(store.clone());
    let lib = LibraryService::new(store.clone(), new_bus()).with_vault(vault.clone());
    let item = lib
        .add(
            "sf",
            LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "typed-secret-value-4")])),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        item.config,
        LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "${SF_TOKEN}")]))
    );
    let updated = lib
        .update(
            &item.id,
            LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "typed-secret-value-5")])),
        )
        .await
        .unwrap();
    assert_eq!(
        updated.config,
        LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "${SF_TOKEN_2}")]))
    );
    assert!(!db_holds(dir.path(), "typed-secret-value"));
}

#[tokio::test]
async fn a_plugins_written_secret_moves_into_personal_as_it_is_synced() {
    let (dir, store) = temp_store().await;
    let vault = Vault::in_memory(store.clone());
    let lib = LibraryService::new(store.clone(), new_bus()).with_vault(vault.clone());
    let root = dir.path().join("forms");
    std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
    std::fs::write(
        root.join(".claude-plugin").join("plugin.json"),
        r#"{"name":"forms"}"#,
    )
    .unwrap();
    std::fs::write(
        root.join(".mcp.json"),
        json!({"mcpServers": {"db": {"command": "node", "env": {"DB_TOKEN": "plugin-secret-6"}}}})
            .to_string(),
    )
    .unwrap();
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
    let server = store
        .find_library_item(LibraryKind::McpServer, "plugin-forms-db")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        server.config,
        LibraryConfig::McpServer(stdio(&[("DB_TOKEN", "${DB_TOKEN}")]))
    );
    assert!(!db_holds(dir.path(), "plugin-secret-6"));
    let mut s = spec("filler");
    s.uses.plugins = vec!["forms".into()];
    let dot = store.create_dot(&s).await.unwrap();
    assert_eq!(dot.spec.variables.0["mcp:plugin-forms-db"], ["DB_TOKEN"]);
}

#[tokio::test]
async fn stored_secrets_move_at_start_and_their_dots_are_granted_the_names() {
    let (dir, store) = temp_store().await;
    // As a version before the vault stored them.
    let item = store
        .add_library_item(
            "sf",
            &LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "stored-secret-value-7")])),
            None,
        )
        .await
        .unwrap();
    let mut s = spec("salesforce");
    s.uses.mcp_servers = vec!["sf".into()];
    let dot = store.create_dot(&s).await.unwrap();
    assert!(dot.spec.variables.0.is_empty());
    let vault = Vault::in_memory(store.clone());
    assert_eq!(vault.move_stored_literals().await.unwrap(), 1);
    assert_eq!(
        store.get_library_item(&item.id).await.unwrap().config,
        LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "${SF_TOKEN}")]))
    );
    assert_eq!(
        store.get_dot(&dot.id).await.unwrap().spec.variables.0["mcp:sf"],
        ["SF_TOKEN"]
    );
    assert!(!db_holds(dir.path(), "stored-secret-value-7"));
    // Nothing is left to move.
    assert_eq!(vault.move_stored_literals().await.unwrap(), 0);
}

#[tokio::test]
async fn the_import_offers_what_windows_sets_once() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store.clone());
    std::env::set_var("DOTS_IMPORT_TEST_TOKEN", "from-windows");
    store
        .add_library_item(
            "imp",
            &LibraryConfig::McpServer(stdio(&[
                ("A", "${DOTS_IMPORT_TEST_TOKEN}"),
                ("B", "${DOTS_IMPORT_TEST_NEVER_SET}"),
            ])),
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        vault.import_candidates().await.unwrap(),
        ["DOTS_IMPORT_TEST_TOKEN"]
    );
    assert!(matches!(
        vault.import(&["PATH".to_string()]).await,
        Err(Error::Invalid(_))
    ));
    assert_eq!(
        vault
            .import(&["DOTS_IMPORT_TEST_TOKEN".to_string()])
            .await
            .unwrap(),
        ["DOTS_IMPORT_TEST_TOKEN"]
    );
    let values = vault
        .personal_values(&["DOTS_IMPORT_TEST_TOKEN".into()])
        .await
        .unwrap();
    assert_eq!(
        values.get("DOTS_IMPORT_TEST_TOKEN").as_deref(),
        Some("from-windows")
    );
    assert!(vault.import_candidates().await.unwrap().is_empty());
}

#[tokio::test]
async fn backups_made_before_the_vault_are_offered_once_and_deleted_on_yes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("dots.db")).await.unwrap();
    let old = old_backup(dir.path());
    let vault = Vault::in_memory(store);
    vault.mark_vault_since().await.unwrap();
    let new = dir
        .path()
        .join("backups")
        .join("dots-0.1.0-2026-10-30-090000.db");
    std::fs::write(&new, b"new").unwrap();
    std::fs::write(dir.path().join("backups").join("notes.txt"), b"x").unwrap();
    assert_eq!(
        vault.old_backups(dir.path()).await.unwrap(),
        ["dots-0.1.0-2026-10-01-090000.db"]
    );
    vault.answer_old_backups(dir.path(), true).await.unwrap();
    assert!(!old.exists() && new.exists());
    assert!(vault.old_backups(dir.path()).await.unwrap().is_empty());
}

#[tokio::test]
async fn keeping_the_old_backups_answers_the_offer_too() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("dots.db")).await.unwrap();
    let old = old_backup(dir.path());
    let vault = Vault::in_memory(store);
    vault.mark_vault_since().await.unwrap();
    vault.answer_old_backups(dir.path(), false).await.unwrap();
    assert!(old.exists());
    assert!(vault.old_backups(dir.path()).await.unwrap().is_empty());
}
