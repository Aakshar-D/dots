//! Where secret values are kept (env vault spec §5.2).

mod common;

use common::temp_store;
use dots_core::vault::{MemorySecrets, SecretStore, MAX_SECRET_BYTES};
use dots_core::Error;

fn round_trip(store: &dyn SecretStore) {
    store.put("personal/GITHUB_TOKEN", "ghp_one").unwrap();
    store.put("personal/SF_TOKEN", "sf-two").unwrap();
    store.put("other/X", "x").unwrap();
    assert_eq!(
        store
            .get("personal/GITHUB_TOKEN")
            .unwrap()
            .as_deref()
            .map(String::as_str),
        Some("ghp_one")
    );
    store.put("personal/GITHUB_TOKEN", "ghp_three").unwrap();
    assert_eq!(
        store
            .get("personal/GITHUB_TOKEN")
            .unwrap()
            .as_deref()
            .map(String::as_str),
        Some("ghp_three")
    );
    assert_eq!(
        store.keys("personal/").unwrap(),
        vec![
            "personal/GITHUB_TOKEN".to_string(),
            "personal/SF_TOKEN".to_string()
        ]
    );
    store.delete("personal/GITHUB_TOKEN").unwrap();
    store.delete("personal/GITHUB_TOKEN").unwrap();
    assert!(store.get("personal/GITHUB_TOKEN").unwrap().is_none());
    let long = "x".repeat(MAX_SECRET_BYTES + 1);
    assert!(matches!(
        store.put("personal/LONG", &long),
        Err(Error::Invalid(_))
    ));
    store
        .put("personal/FULL", &"y".repeat(MAX_SECRET_BYTES))
        .unwrap();
    for key in store.keys("").unwrap() {
        store.delete(&key).unwrap();
    }
    assert!(store.keys("").unwrap().is_empty());
}

#[test]
fn memory_secrets_keep_replace_list_and_delete_values() {
    round_trip(&MemorySecrets::new());
}

#[cfg(windows)]
#[test]
fn credential_manager_keeps_replace_lists_and_deletes_values() {
    use dots_core::vault::WindowsCredentials;
    let install = format!("test-{}", dots_core::util::new_id());
    round_trip(&WindowsCredentials::new(&install));
}

#[cfg(windows)]
#[test]
fn two_installs_never_see_each_others_secrets() {
    use dots_core::vault::WindowsCredentials;
    let a = WindowsCredentials::new(&format!("test-{}", dots_core::util::new_id()));
    let b = WindowsCredentials::new(&format!("test-{}", dots_core::util::new_id()));
    a.put("personal/SHARED_NAME", "from-a").unwrap();
    assert!(b.get("personal/SHARED_NAME").unwrap().is_none());
    assert!(b.keys("").unwrap().is_empty());
    a.delete("personal/SHARED_NAME").unwrap();
}

#[tokio::test]
async fn the_install_id_is_made_once_and_kept() {
    let (dir, store) = temp_store().await;
    let first = store.install_id().await.unwrap();
    assert_eq!(first.len(), 26);
    assert_eq!(store.install_id().await.unwrap(), first);
    store.close().await;
    let again = dots_core::store::Store::open(&dir.path().join("dots.db"))
        .await
        .unwrap();
    assert_eq!(again.install_id().await.unwrap(), first);
    // The settings the person edits do not carry it.
    let settings = serde_json::to_value(again.get_settings().await.unwrap()).unwrap();
    assert!(settings.get("install_id").is_none());
}
