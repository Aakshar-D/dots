//! API keys: making them and the table that holds their hashes (accounts spec §10).

mod common;

use common::temp_store;
use dots_core::accounts::api_keys::{key_prefix, looks_like_key, new_api_key, API_KEY_PREFIX};
use dots_core::store::ApiKeyRow;
use dots_core::util::{new_id, token_hash};
use dots_core::Error;

fn row_for(key: &str, scope: &str, at: &str) -> ApiKeyRow {
    ApiKeyRow {
        id: new_id(),
        scope: scope.into(),
        workspace_id: "default".into(),
        name: "CI".into(),
        prefix: key_prefix(key),
        hash: token_hash(key),
        created_at: at.into(),
        last_used_at: None,
        revoked_at: None,
    }
}

#[test]
fn new_keys_have_the_shape() {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..100 {
        let key = new_api_key();
        assert!(key.starts_with(API_KEY_PREFIX));
        assert_eq!(key.len(), 51);
        assert!(key[API_KEY_PREFIX.len()..]
            .chars()
            .all(|c| c.is_ascii_alphanumeric()));
        assert!(seen.insert(key));
    }
}

#[test]
fn key_prefix_is_12_chars() {
    let key = new_api_key();
    assert_eq!(key_prefix(&key), key[..12]);
    assert_eq!(key_prefix(&key).len(), 12);
}

#[test]
fn looks_like_key_accepts_new_keys_only() {
    let key = new_api_key();
    assert!(looks_like_key(&key));
    assert!(!looks_like_key(&key[..50]));
    assert!(!looks_like_key(&format!("{key}a")));
    assert!(!looks_like_key(&format!("dots_pk_{}", &key[8..])));
    assert!(!looks_like_key(&format!("{}-{}", &key[..20], &key[21..])));
    assert!(!looks_like_key(""));
}

#[tokio::test]
async fn add_list_revoke_touch_round_trip() {
    let (_dir, store) = temp_store().await;
    let (k1, k2) = (new_api_key(), new_api_key());
    let first = row_for(&k1, "personal", "2026-10-30T09:00:00.000Z");
    let second = row_for(&k2, "personal", "2026-10-30T10:00:00.000Z");
    let other = row_for(&new_api_key(), "org1", "2026-10-30T11:00:00.000Z");
    for r in [&first, &second, &other] {
        store.add_api_key(r).await.unwrap();
    }
    assert!(matches!(
        store.add_api_key(&first).await,
        Err(Error::Conflict(_))
    ));

    let listed = store.api_keys("personal").await.unwrap();
    assert_eq!(listed, vec![second.clone(), first.clone()]);
    assert_eq!(store.api_key(&first.id).await.unwrap(), Some(first.clone()));
    assert_eq!(store.api_key("nope").await.unwrap(), None);

    store
        .touch_api_key(&first.id, "2026-10-30T12:00:00.000Z")
        .await
        .unwrap();
    store
        .revoke_api_key(&first.id, "2026-10-30T13:00:00.000Z")
        .await
        .unwrap();
    store
        .revoke_api_key(&first.id, "2026-10-30T14:00:00.000Z")
        .await
        .unwrap();
    let got = store.api_key(&first.id).await.unwrap().unwrap();
    assert_eq!(
        got.last_used_at.as_deref(),
        Some("2026-10-30T12:00:00.000Z")
    );
    assert_eq!(got.revoked_at.as_deref(), Some("2026-10-30T13:00:00.000Z"));

    assert!(matches!(
        store
            .revoke_api_key("nope", "2026-10-30T13:00:00.000Z")
            .await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn lookup_by_secret_finds_only_the_exact_key() {
    let (_dir, store) = temp_store().await;
    let key = new_api_key();
    let row = row_for(&key, "personal", "2026-10-30T09:00:00.000Z");
    store.add_api_key(&row).await.unwrap();
    assert_eq!(store.api_key_by_secret(&key).await.unwrap(), Some(row));

    let mut changed = key.clone().into_bytes();
    let last = changed.len() - 1;
    changed[last] = if changed[last] == b'a' { b'b' } else { b'a' };
    let changed = String::from_utf8(changed).unwrap();
    assert_eq!(store.api_key_by_secret(&changed).await.unwrap(), None);
}

#[tokio::test]
async fn a_key_for_a_workspace_is_added_only_while_the_workspace_is_there() {
    let (_dir, store) = temp_store().await;
    let key = new_api_key();
    let mut row = row_for(&key, "personal", "2026-10-30T09:00:00.000Z");
    row.workspace_id = "gone".into();
    assert!(matches!(
        store.add_workspace_api_key(&row).await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(store.api_key(&row.id).await.unwrap(), None);

    row.workspace_id = "default".into();
    store.add_workspace_api_key(&row).await.unwrap();
    assert_eq!(store.api_key(&row.id).await.unwrap(), Some(row));
}
