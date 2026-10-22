//! Secret values typed into the Library move into Personal, and a setup from before the vault
//! is brought in (env vault spec §11).

mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use common::secrets::FailingSecrets;
use common::{spec, temp_store};
use dots_core::events::new_bus;
use dots_core::library::{LibraryConfig, LibraryKind, McpServerConfig, PluginConfig};
use dots_core::library_service::LibraryService;
use dots_core::store::Store;
use dots_core::vault::{redact, EntryKind, Vault};
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
        LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "${SF_TOKEN}")]))
    );
    assert!(!db_holds(dir.path(), "typed-secret-value"));
}

#[tokio::test]
async fn a_form_edit_of_a_name_only_its_server_reads_replaces_the_value() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store.clone());
    let lib = LibraryService::new(store.clone(), new_bus()).with_vault(vault.clone());
    let item = lib
        .add(
            "sf",
            LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "old-rotated-value-23")])),
            None,
        )
        .await
        .unwrap();
    let mut s = spec("rotates");
    s.uses.mcp_servers = vec!["sf".into()];
    let dot = store.create_dot(&s).await.unwrap();
    let updated = lib
        .update(
            &item.id,
            LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "new-rotated-value-24")])),
        )
        .await
        .unwrap();
    assert_eq!(
        updated.config,
        LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "${SF_TOKEN}")]))
    );
    let names: Vec<String> = vault
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["SF_TOKEN"]);
    let values = vault.personal_values(&["SF_TOKEN".into()]).await.unwrap();
    assert_eq!(
        values.get("SF_TOKEN").as_deref(),
        Some("new-rotated-value-24")
    );
    assert_eq!(
        store.get_dot(&dot.id).await.unwrap().spec.variables.0["mcp:sf"],
        ["SF_TOKEN"]
    );
}

#[tokio::test]
async fn a_form_edit_of_a_shared_name_moves_under_a_new_one() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store.clone());
    let lib = LibraryService::new(store.clone(), new_bus()).with_vault(vault.clone());
    let item = lib
        .add(
            "sf",
            LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "shared-value-25")])),
            None,
        )
        .await
        .unwrap();
    lib.add(
        "sf-reports",
        LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "${SF_TOKEN}")])),
        None,
    )
    .await
    .unwrap();
    let updated = lib
        .update(
            &item.id,
            LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "edited-value-26")])),
        )
        .await
        .unwrap();
    assert_eq!(
        updated.config,
        LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "${SF_TOKEN_2}")]))
    );
    let values = vault
        .personal_values(&["SF_TOKEN".into(), "SF_TOKEN_2".into()])
        .await
        .unwrap();
    assert_eq!(values.get("SF_TOKEN").as_deref(), Some("shared-value-25"));
    assert_eq!(values.get("SF_TOKEN_2").as_deref(), Some("edited-value-26"));
}

#[tokio::test]
async fn a_plugin_resync_with_another_value_keeps_the_name_and_the_grant() {
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
    let mcp = |value: &str| {
        json!({"mcpServers": {"db": {"command": "node", "env": {"SF_TOKEN": value}}}}).to_string()
    };
    std::fs::write(root.join(".mcp.json"), mcp("plugin-first-value-21")).unwrap();
    let plugin = LibraryConfig::Plugin(PluginConfig {
        path: root.to_string_lossy().into_owned(),
        installed_id: None,
        run_hooks: false,
    });
    let item = lib.add("forms", plugin.clone(), None).await.unwrap();
    let mut s = spec("forms-user");
    s.uses.plugins = vec!["forms".into()];
    let dot = store.create_dot(&s).await.unwrap();
    assert_eq!(dot.spec.variables.0["mcp:plugin-forms-db"], ["SF_TOKEN"]);
    // The plugin's file now has another value; the plugin is synced again.
    std::fs::write(root.join(".mcp.json"), mcp("plugin-second-value-22")).unwrap();
    lib.update(&item.id, plugin).await.unwrap();
    let server = store
        .find_library_item(LibraryKind::McpServer, "plugin-forms-db")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        server.config,
        LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "${SF_TOKEN}")]))
    );
    // Personal wins: its value stays, and no other name is made.
    let names: Vec<String> = vault
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["SF_TOKEN"]);
    let values = vault.personal_values(&["SF_TOKEN".into()]).await.unwrap();
    assert_eq!(
        values.get("SF_TOKEN").as_deref(),
        Some("plugin-first-value-21")
    );
    // The dot is still granted every name the server reads.
    let grants = store.get_dot(&dot.id).await.unwrap().spec.variables;
    assert_eq!(grants.0["mcp:plugin-forms-db"], ["SF_TOKEN"]);
    // The plugin's other value is still scrubbed.
    assert_eq!(
        redact::global().scrub("plugin-second-value-22").as_ref(),
        "[secret:SF_TOKEN]"
    );
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
    std::env::remove_var("DOTS_IMPORT_TEST_TOKEN");
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

#[tokio::test]
async fn the_clis_own_and_the_base_variables_are_never_offered_or_imported() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store.clone());
    // A prefix of the CLI's own, a name in CLI_ENV and a name in BASE_ENV, all set for Dots.
    let own = [
        "ANTHROPIC_DOTS_TEST_KEY",
        "CLAUDE_CONFIG_DIR",
        "USERPROFILE",
    ];
    let before: Vec<_> = own.iter().map(std::env::var_os).collect();
    for n in own {
        std::env::set_var(n, "made-up-value-for-the-test");
    }
    std::env::set_var("DOTS_IMPORT_TEST_OTHER", "made-up-value-for-the-test");
    store
        .add_library_item(
            "reads",
            &LibraryConfig::McpServer(stdio(&[
                ("A", "${ANTHROPIC_DOTS_TEST_KEY}"),
                ("B", "${CLAUDE_CONFIG_DIR}"),
                ("C", "${USERPROFILE}"),
                ("D", "${DOTS_IMPORT_TEST_OTHER}"),
            ])),
            None,
        )
        .await
        .unwrap();
    let offered = vault.import_candidates().await;
    let mut refused = Vec::new();
    for n in own {
        refused.push(vault.import(&[n.to_string()]).await);
    }
    let names: Vec<String> = own.iter().map(|n| n.to_string()).collect();
    let personal = vault.personal_values(&names).await;
    std::env::remove_var("DOTS_IMPORT_TEST_OTHER");
    for (n, old) in own.iter().zip(before) {
        match old {
            Some(v) => std::env::set_var(n, v),
            None => std::env::remove_var(n),
        }
    }
    assert_eq!(offered.unwrap(), ["DOTS_IMPORT_TEST_OTHER"]);
    assert!(refused.iter().all(|r| matches!(r, Err(Error::Invalid(_)))));
    let personal = personal.unwrap();
    assert!(own.iter().all(|n| personal.get(n).is_none()));
}

#[tokio::test]
async fn one_failing_server_does_not_stop_the_others_or_the_vacuum() {
    let (dir, store) = temp_store().await;
    // A value the secret store fails to keep, listed before the good one.
    let bad = store
        .add_library_item(
            "aa-bad",
            &LibraryConfig::McpServer(stdio(&[("BAD_TOKEN", "bad-value-secret-9")])),
            None,
        )
        .await
        .unwrap();
    let good = store
        .add_library_item(
            "zz-good",
            &LibraryConfig::McpServer(stdio(&[("GOOD_TOKEN", "good-secret-value-10")])),
            None,
        )
        .await
        .unwrap();
    let vault = Vault::new(store.clone(), Arc::new(FailingSecrets::new("BAD_TOKEN")));
    // The failing server is logged by name and left for the next start.
    assert_eq!(vault.move_stored_literals().await.unwrap(), 1);
    assert_eq!(
        store.get_library_item(&bad.id).await.unwrap().config,
        LibraryConfig::McpServer(stdio(&[("BAD_TOKEN", "bad-value-secret-9")]))
    );
    assert_eq!(
        store.get_library_item(&good.id).await.unwrap().config,
        LibraryConfig::McpServer(stdio(&[("GOOD_TOKEN", "${GOOD_TOKEN}")]))
    );
    assert!(!db_holds(dir.path(), "good-secret-value-10"));
    // The pending marker is cleared once the vacuum ran.
    assert_eq!(
        store.marker("vault_vacuum_pending").await.unwrap(),
        Some(json!(false))
    );
}

#[tokio::test]
async fn a_pending_vacuum_runs_at_the_next_start() {
    let (dir, store) = temp_store().await;
    let item = store
        .add_library_item(
            "sf",
            &LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "pending-secret-value-11")])),
            None,
        )
        .await
        .unwrap();
    assert!(db_holds(dir.path(), "pending-secret-value-11"));
    // As a start that moved the server and stopped before the vacuum left it.
    let moved = LibraryConfig::McpServer(stdio(&[("SF_TOKEN", "${SF_TOKEN}")]));
    store.update_library_item(&item.id, &moved).await.unwrap();
    store
        .set_marker("vault_vacuum_pending", &json!(true), false)
        .await
        .unwrap();
    let vault = Vault::in_memory(store.clone());
    assert_eq!(vault.move_stored_literals().await.unwrap(), 0);
    assert!(!db_holds(dir.path(), "pending-secret-value-11"));
    assert_eq!(
        store.marker("vault_vacuum_pending").await.unwrap(),
        Some(json!(false))
    );
}

#[tokio::test]
async fn a_key_with_no_valid_name_moves_under_a_derived_one() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store);
    let (config, moved) = vault
        .move_literals("p", &stdio(&[("1PASSWORD_TOKEN", "digit-secret-12")]))
        .await
        .unwrap();
    assert_eq!(moved, ["_1PASSWORD_TOKEN"]);
    assert_eq!(config, stdio(&[("1PASSWORD_TOKEN", "${_1PASSWORD_TOKEN}")]));
    // The same value finds the same name again.
    let (_, again) = vault
        .move_literals("q", &stdio(&[("1PASSWORD_TOKEN", "digit-secret-12")]))
        .await
        .unwrap();
    assert_eq!(again, moved);
}

#[tokio::test]
async fn a_value_too_long_to_move_stays_but_is_scrubbed() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store);
    let long = format!("oversize-{}", "z".repeat(3000));
    let config = stdio(&[("BIG_TOKEN", &long)]);
    let (kept, moved) = vault.move_literals("big", &config).await.unwrap();
    assert!(moved.is_empty());
    assert_eq!(kept, config);
    let text = format!("log: {long}");
    let scrubbed = redact::global().scrub(&text);
    assert!(!scrubbed.contains("zzzzzzzz"), "value was not scrubbed");
}

#[tokio::test]
async fn a_taken_name_gets_a_suffix_within_128_characters() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store);
    let key = format!("{}_TOKEN", "A".repeat(122));
    assert_eq!(key.len(), 128);
    vault
        .add(&key, EntryKind::Secret, "first-value-13")
        .await
        .unwrap();
    let (_, moved) = vault
        .move_literals("s", &stdio(&[(&key, "second-value-14")]))
        .await
        .unwrap();
    assert_eq!(moved.len(), 1);
    assert_eq!(moved[0].len(), 128);
    assert!(moved[0].ends_with("_2"));
}

#[tokio::test]
async fn a_name_whose_secret_is_missing_counts_as_taken() {
    let (_dir, store) = temp_store().await;
    store
        .add_env_entry("SF_TOKEN", EntryKind::Secret, None)
        .await
        .unwrap();
    let vault = Vault::in_memory(store);
    let (_, moved) = vault
        .move_literals("s", &stdio(&[("SF_TOKEN", "fresh-value-15")]))
        .await
        .unwrap();
    assert_eq!(moved, ["SF_TOKEN_2"]);
}

#[tokio::test]
async fn a_one_word_authorization_keeps_its_shape() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store);
    let (config, moved) = vault
        .move_literals("api", &http(&[("Authorization", "bare-token-value-16")]))
        .await
        .unwrap();
    assert_eq!(moved, ["API_AUTHORIZATION"]);
    assert_eq!(config, http(&[("Authorization", "${API_AUTHORIZATION}")]));
}

#[tokio::test]
async fn an_old_backup_that_cannot_be_deleted_does_not_stop_the_answer() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("dots.db")).await.unwrap();
    let old = old_backup(dir.path());
    let vault = Vault::in_memory(store);
    vault.mark_vault_since().await.unwrap();
    // A backup another process holds open (no delete sharing) cannot be removed.
    let stuck = dir.path().join("backups").join("dots-0.0.9-stuck.db");
    std::fs::write(&stuck, b"held").unwrap();
    let held = {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::File::options()
            .write(true)
            .share_mode(3)
            .open(&stuck)
            .unwrap()
    };
    held.set_modified(SystemTime::now() - Duration::from_secs(3600))
        .unwrap();
    vault.answer_old_backups(dir.path(), true).await.unwrap();
    assert!(!old.exists() && stuck.exists());
    assert!(vault.old_backups(dir.path()).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_plain_url_or_a_path_stays_where_it_is() {
    let (_dir, store) = temp_store().await;
    let vault = Vault::in_memory(store);
    let config = stdio(&[
        ("AUTH", "https://login.example.com/realms/acme"),
        (
            "GOOGLE_APPLICATION_CREDENTIALS",
            r"C:\keys\service-account.json",
        ),
        ("SSH_KEY", "~/.ssh/id_ed25519"),
        ("AWS_CREDENTIALS", "/home/me/.aws/credentials"),
        ("TOKEN_URL", "https://login.example.com/token"),
        ("DB_PASSWORD", "https://admin:url-secret-17@db.example.com"),
    ]);
    let (kept, moved) = vault.move_literals("u", &config).await.unwrap();
    assert_eq!(moved, ["DB_PASSWORD"]);
    assert_eq!(
        kept,
        stdio(&[
            ("AUTH", "https://login.example.com/realms/acme"),
            (
                "GOOGLE_APPLICATION_CREDENTIALS",
                r"C:\keys\service-account.json"
            ),
            ("SSH_KEY", "~/.ssh/id_ed25519"),
            ("AWS_CREDENTIALS", "/home/me/.aws/credentials"),
            ("TOKEN_URL", "https://login.example.com/token"),
            ("DB_PASSWORD", "${DB_PASSWORD}"),
        ])
    );
    // A base64 secret that starts with `/` is not a path.
    let (_, moved) = vault
        .move_literals("b", &stdio(&[("API_KEY", "/aB3dE+fG7hI/jK9lM=")]))
        .await
        .unwrap();
    assert_eq!(moved, ["API_KEY"]);
}

#[tokio::test]
async fn a_value_that_cannot_move_never_sets_the_vacuum_marker() {
    let (_dir, store) = temp_store().await;
    let long = format!("unmovable-{}", "u".repeat(3000));
    let item = store
        .add_library_item(
            "big",
            &LibraryConfig::McpServer(stdio(&[("BIG_TOKEN", &long)])),
            None,
        )
        .await
        .unwrap();
    // Two starts: nothing can move, so neither rewrites a config or vacuums.
    for _ in 0..2 {
        let vault = Vault::in_memory(store.clone());
        assert_eq!(vault.move_stored_literals().await.unwrap(), 0);
        assert_eq!(store.marker("vault_vacuum_pending").await.unwrap(), None);
    }
    assert_eq!(
        store.get_library_item(&item.id).await.unwrap().config,
        LibraryConfig::McpServer(stdio(&[("BIG_TOKEN", &long)]))
    );
}

#[tokio::test]
async fn a_value_the_vault_refuses_stays_but_is_scrubbed() {
    let (_dir, store) = temp_store().await;
    let refused = "refused\0value-secret-18";
    let item = store
        .add_library_item(
            "nul",
            &LibraryConfig::McpServer(stdio(&[("NUL_TOKEN", refused)])),
            None,
        )
        .await
        .unwrap();
    let vault = Vault::in_memory(store.clone());
    assert_eq!(vault.move_stored_literals().await.unwrap(), 0);
    assert_eq!(
        store.get_library_item(&item.id).await.unwrap().config,
        LibraryConfig::McpServer(stdio(&[("NUL_TOKEN", refused)]))
    );
    assert_eq!(store.marker("vault_vacuum_pending").await.unwrap(), None);
    assert_eq!(
        redact::global().scrub(&format!("log: {refused}")).as_ref(),
        "log: [secret:NUL_TOKEN]"
    );
}

#[tokio::test]
async fn the_vault_since_mark_is_written_despite_a_failing_server() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    std::fs::create_dir_all(&data).unwrap();
    let good = {
        let store = Store::open(&data.join("dots.db")).await.unwrap();
        store
            .add_library_item(
                "aa-bad",
                &LibraryConfig::McpServer(stdio(&[("BAD_TOKEN", "bad-start-secret-19")])),
                None,
            )
            .await
            .unwrap();
        store
            .add_library_item(
                "zz-good",
                &LibraryConfig::McpServer(stdio(&[("GOOD_TOKEN", "good-start-secret-20")])),
                None,
            )
            .await
            .unwrap()
    };
    let mut cfg = dots_core::Config::new(&data);
    cfg.port = 0;
    cfg.secrets = Some(Arc::new(FailingSecrets::new("BAD_TOKEN")));
    let rt = dots_core::Runtime::start(cfg).await.unwrap();
    assert!(rt.store().marker("vault_since").await.unwrap().is_some());
    assert_eq!(
        rt.store().get_library_item(&good.id).await.unwrap().config,
        LibraryConfig::McpServer(stdio(&[("GOOD_TOKEN", "${GOOD_TOKEN}")]))
    );
    rt.shutdown();
}
