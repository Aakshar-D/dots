use dots_core::accounts::{ensure_identity, identity, set_profile, IDENTITY_SECRET};
use dots_core::store::Store;
use dots_core::vault::org::key_fingerprint;
use dots_core::vault::{MemorySecrets, SecretStore};
use dots_core::Error;

async fn store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("dots.db")).await.unwrap();
    (dir, store)
}

fn invalid(r: dots_core::Result<dots_core::accounts::Identity>) -> String {
    match r {
        Err(Error::Invalid(m)) => m,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[tokio::test]
async fn makes_identity_once() {
    let (_dir, store) = store().await;
    let secrets = MemorySecrets::new();
    let first = ensure_identity(&store, &secrets).await.unwrap();
    assert!(!first.public_key.is_empty());
    assert_eq!(
        first.fingerprint,
        key_fingerprint(&first.public_key).unwrap()
    );
    assert_eq!((first.name.as_str(), first.email.as_str()), ("", ""));
    let secret = secrets.get(IDENTITY_SECRET).unwrap().unwrap();
    minisign::SecretKeyBox::from_string(&secret).unwrap();
    let second = ensure_identity(&store, &secrets).await.unwrap();
    assert_eq!(second.public_key, first.public_key);
}

#[tokio::test]
async fn new_pair_when_secret_lost() {
    let (_dir, store) = store().await;
    let secrets = MemorySecrets::new();
    let first = ensure_identity(&store, &secrets).await.unwrap();
    secrets.delete(IDENTITY_SECRET).unwrap();
    let second = ensure_identity(&store, &secrets).await.unwrap();
    assert_ne!(second.public_key, first.public_key);
    assert!(secrets.get(IDENTITY_SECRET).unwrap().is_some());
}

#[tokio::test]
async fn profile_trims_and_validates() {
    let (_dir, store) = store().await;
    let secrets = MemorySecrets::new();
    ensure_identity(&store, &secrets).await.unwrap();
    let set = set_profile(&store, "  Asha Rao ", " asha@acme.com ")
        .await
        .unwrap();
    assert_eq!(
        (set.name.as_str(), set.email.as_str()),
        ("Asha Rao", "asha@acme.com")
    );
    assert_eq!(
        invalid(set_profile(&store, "", "a@b.c").await),
        "a name is needed"
    );
    assert_eq!(
        invalid(set_profile(&store, "   ", "a@b.c").await),
        "a name is needed"
    );
    assert_eq!(
        invalid(set_profile(&store, "Asha", "asha").await),
        "an email address needs an @"
    );
    assert_eq!(
        invalid(set_profile(&store, "Asha", "a@").await),
        "an email address needs an @"
    );
    assert_eq!(
        invalid(set_profile(&store, "Asha", "@b.c").await),
        "an email address needs an @"
    );
    assert_eq!(
        invalid(set_profile(&store, &"a".repeat(81), "a@b.c").await),
        "a name must be at most 80 characters"
    );
    let long = format!("{}@b.c", "a".repeat(251));
    assert_eq!(
        invalid(set_profile(&store, "Asha", &long).await),
        "an email address must be at most 254 characters"
    );
    let now = identity(&store).await.unwrap();
    assert_eq!(
        (now.name.as_str(), now.email.as_str()),
        ("Asha Rao", "asha@acme.com")
    );
}
