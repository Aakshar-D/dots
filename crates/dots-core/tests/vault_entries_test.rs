//! Personal variables (env vault spec §5): plain values in the database, secrets in the secret
//! store, and no listing that carries a secret's value.

mod common;

use std::sync::Arc;

use common::temp_store;
use dots_core::vault::{EntryKind, MemorySecrets, SecretStore, Vault, MAX_SECRET_BYTES};
use dots_core::Error;

async fn vault() -> (tempfile::TempDir, Vault, Arc<MemorySecrets>) {
    let (dir, store) = temp_store().await;
    let secrets = Arc::new(MemorySecrets::new());
    (dir, Vault::new(store, secrets.clone()), secrets)
}

#[tokio::test]
async fn a_secret_goes_to_the_secret_store_and_is_never_listed() {
    let (_dir, vault, secrets) = vault().await;
    vault
        .add("LOG_LEVEL", EntryKind::Plain, "debug")
        .await
        .unwrap();
    let added = vault
        .add("GITHUB_TOKEN", EntryKind::Secret, "ghp_s3cret")
        .await
        .unwrap();
    assert_eq!(added.value, None);
    let listed = vault.list().await.unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|e| (e.name.as_str(), e.value.as_deref()))
            .collect::<Vec<_>>(),
        vec![("GITHUB_TOKEN", None), ("LOG_LEVEL", Some("debug"))]
    );
    assert!(!serde_json::to_string(&listed)
        .unwrap()
        .contains("ghp_s3cret"));
    assert_eq!(
        secrets
            .get("personal/GITHUB_TOKEN")
            .unwrap()
            .as_deref()
            .map(String::as_str),
        Some("ghp_s3cret")
    );
}

#[tokio::test]
async fn a_name_taken_in_any_case_is_refused() {
    let (_dir, vault, _) = vault().await;
    vault
        .add("GITHUB_TOKEN", EntryKind::Secret, "a")
        .await
        .unwrap();
    let err = vault
        .add("github_token", EntryKind::Plain, "b")
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Conflict(m) if m == "A variable named github_token is already in Personal."),
        "{err}"
    );
}

#[tokio::test]
async fn names_and_values_a_child_process_cannot_take_are_refused() {
    let (_dir, vault, secrets) = vault().await;
    let long_name = "A".repeat(129);
    for name in [
        "",
        "1TOKEN",
        "MY-TOKEN",
        "MY TOKEN",
        "Ü",
        long_name.as_str(),
    ] {
        assert!(
            matches!(
                vault.add(name, EntryKind::Plain, "x").await,
                Err(Error::Invalid(_))
            ),
            "{name:?}"
        );
    }
    vault
        .add(&"A".repeat(128), EntryKind::Plain, "x")
        .await
        .unwrap();
    vault.add("_PRIVATE", EntryKind::Plain, "").await.unwrap();
    assert!(matches!(
        vault.add("NUL", EntryKind::Plain, "a\0b").await,
        Err(Error::Invalid(_))
    ));
    assert!(matches!(
        vault.add("EMPTY", EntryKind::Secret, "").await,
        Err(Error::Invalid(_))
    ));
    let big = "x".repeat(32 * 1024 + 1);
    assert!(matches!(
        vault.add("BIG", EntryKind::Plain, &big).await,
        Err(Error::Invalid(_))
    ));
    // A secret too long for the store leaves no row behind.
    let long = "x".repeat(MAX_SECRET_BYTES + 1);
    assert!(matches!(
        vault.add("LONG", EntryKind::Secret, &long).await,
        Err(Error::Invalid(_))
    ));
    assert!(!vault.list().await.unwrap().iter().any(|e| e.name == "LONG"));
    assert!(secrets.keys("").unwrap().is_empty());
}

#[tokio::test]
async fn a_value_is_replaced_and_a_removed_secret_leaves_the_store() {
    let (_dir, vault, secrets) = vault().await;
    vault
        .add("SF_TOKEN", EntryKind::Secret, "old")
        .await
        .unwrap();
    vault
        .add("MODE", EntryKind::Plain, "readonly")
        .await
        .unwrap();
    let replaced = vault.set_value("sf_token", "new").await.unwrap();
    assert_eq!((replaced.name.as_str(), replaced.value), ("SF_TOKEN", None));
    assert_eq!(
        secrets
            .get("personal/SF_TOKEN")
            .unwrap()
            .as_deref()
            .map(String::as_str),
        Some("new")
    );
    assert_eq!(
        vault
            .set_value("MODE", "write")
            .await
            .unwrap()
            .value
            .as_deref(),
        Some("write")
    );
    assert!(matches!(
        vault.set_value("SF_TOKEN", "").await,
        Err(Error::Invalid(_))
    ));
    vault.remove("SF_TOKEN").await.unwrap();
    assert!(secrets.get("personal/SF_TOKEN").unwrap().is_none());
    assert!(matches!(
        vault.remove("SF_TOKEN").await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        vault.set_value("GONE", "x").await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn the_runtime_keeps_a_tests_secrets_in_memory() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = dots_core::Config::new(dir.path().join("data"));
    // A missing program: no test may run a `claude` found on the PATH.
    cfg.claude_path = Some(dir.path().join("no-claude.exe"));
    assert!(cfg.secrets.is_some());
    assert!(format!("{:?}", cfg.secrets).contains("MemorySecrets"));
    let settings = dots_core::settings::Settings::default();
    assert!(dots_core::Config::from_settings(dir.path(), &settings)
        .secrets
        .is_none());
    let rt = dots_core::Runtime::start(cfg).await.unwrap();
    rt.vault()
        .add("A_TOKEN", EntryKind::Secret, "x")
        .await
        .unwrap();
    assert_eq!(rt.vault().list().await.unwrap().len(), 1);
    rt.shutdown();
}

#[tokio::test]
async fn a_secret_row_with_a_value_is_refused_by_the_database() {
    let (_dir, store) = temp_store().await;
    let refused = store
        .add_env_entry("LEAK", EntryKind::Secret, Some("ghp_s3cret"))
        .await;
    assert!(refused.is_err());
    assert!(store.list_env_entries().await.unwrap().is_empty());
}
