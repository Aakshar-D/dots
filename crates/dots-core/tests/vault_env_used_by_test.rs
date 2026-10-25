//! What the Env tab shows of each variable beside its value (env vault spec §9, deferred by
//! plan A §14): "used by", the dots of the variable's own scope that grant it, and, for a
//! Personal secret, whether its value is gone from the secret store. Neither reads a value, and
//! no listing carries one.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use common::orgs::{OrgsHarness, ACME, TOKEN};
use common::spec;
use dots_core::library::{LibraryConfig, McpServerConfig};
use dots_core::model::{Dot, DotRef, DotSpec};
use dots_core::store::Store;
use dots_core::vault::{EntryKind, Grants, MemorySecrets, SecretStore, Vault};
use serde_json::json;
use zeroize::Zeroizing;

const PERSONAL_TOKEN: &str = "personal-secret-gh-token-5d21a7";
const ORG_KEY: &str = "org-secret-api-key-93be04";
const ORG_URL: &str = "https://api.acme.example";

/// A secret store that counts the values read from it, so a test can tell that a listing
/// asked only for the keys.
#[derive(Debug, Default)]
struct Counting {
    inner: MemorySecrets,
    gets: AtomicUsize,
}

impl SecretStore for Counting {
    fn put(&self, key: &str, value: &str) -> dots_core::Result<()> {
        self.inner.put(key, value)
    }
    fn get(&self, key: &str) -> dots_core::Result<Option<Zeroizing<String>>> {
        self.gets.fetch_add(1, Ordering::SeqCst);
        self.inner.get(key)
    }
    fn delete(&self, key: &str) -> dots_core::Result<()> {
        self.inner.delete(key)
    }
    fn keys(&self, prefix: &str) -> dots_core::Result<Vec<String>> {
        self.inner.keys(prefix)
    }
}

fn granting(name: &str, grants: &[(&str, &[&str])]) -> DotSpec {
    let mut s = spec(name);
    s.variables = Grants(
        grants
            .iter()
            .map(|(consumer, names)| {
                let names = names.iter().map(|n| n.to_string()).collect();
                (consumer.to_string(), names)
            })
            .collect(),
    );
    s
}

fn named(dot: &Dot) -> DotRef {
    DotRef {
        id: dot.id.clone(),
        name: dot.spec.name.clone(),
    }
}

async fn github_server(store: &Store) {
    let config = McpServerConfig::Stdio {
        command: "gh-mcp".into(),
        args: Vec::new(),
        cwd: None,
        env: [("TOKEN".to_string(), "${GITHUB_TOKEN}".to_string())].into(),
    };
    store
        .add_library_item("github", &LibraryConfig::McpServer(config), None)
        .await
        .unwrap();
}

#[tokio::test]
async fn used_by_lists_the_personal_dots_that_grant_the_name_in_any_case() {
    let h = OrgsHarness::new().await;
    let vault = h.vault();
    github_server(&h.store).await;
    vault
        .add("GITHUB_TOKEN", EntryKind::Secret, PERSONAL_TOKEN)
        .await
        .unwrap();
    vault
        .add("LOG_LEVEL", EntryKind::Plain, "debug")
        .await
        .unwrap();
    vault.add("UNUSED", EntryKind::Plain, "x").await.unwrap();

    // Ticking the server grants it GITHUB_TOKEN; the shell grant is in another case.
    let mut uses_github = spec("nightly");
    uses_github.uses.mcp_servers = vec!["github".into()];
    let nightly = h.store.create_dot(&uses_github, None).await.unwrap();
    let triage = h
        .store
        .create_dot(
            &granting("triage", &[("shell", &["github_token", "LOG_LEVEL"])]),
            None,
        )
        .await
        .unwrap();
    h.store.create_dot(&spec("idle"), None).await.unwrap();
    // A dot of an org grants the same names: it reads them from the org, not from Personal.
    uses_github.name = "acme-bot".into();
    h.store.create_dot(&uses_github, Some(ACME)).await.unwrap();
    h.store
        .create_dot(
            &granting("acme-shell", &[("shell", &["LOG_LEVEL"])]),
            Some(ACME),
        )
        .await
        .unwrap();

    let listed = vault.list().await.unwrap();
    let used: Vec<(&str, Vec<DotRef>)> = listed
        .iter()
        .map(|e| (e.name.as_str(), e.used_by.clone()))
        .collect();
    assert_eq!(
        used,
        [
            ("GITHUB_TOKEN", vec![named(&nightly), named(&triage)]),
            ("LOG_LEVEL", vec![named(&triage)]),
            ("UNUSED", vec![]),
        ]
    );
}

#[tokio::test]
async fn value_missing_is_true_only_for_a_secret_whose_value_is_gone() {
    let (_dir, store) = common::temp_store().await;
    let secrets = Arc::new(Counting::default());
    let vault = Vault::new(store, secrets.clone());
    vault
        .add("GITHUB_TOKEN", EntryKind::Secret, PERSONAL_TOKEN)
        .await
        .unwrap();
    vault
        .add("NPM_TOKEN", EntryKind::Secret, "personal-secret-npm-3c7e19")
        .await
        .unwrap();
    vault
        .add("LOG_LEVEL", EntryKind::Plain, "debug")
        .await
        .unwrap();
    let missing = |listed: &[dots_core::vault::EnvEntry]| -> Vec<(String, bool)> {
        listed
            .iter()
            .map(|e| (e.name.clone(), e.value_missing))
            .collect()
    };
    let gets = secrets.gets.load(Ordering::SeqCst);
    assert_eq!(
        missing(&vault.list().await.unwrap()),
        [
            ("GITHUB_TOKEN".to_string(), false),
            ("LOG_LEVEL".to_string(), false),
            ("NPM_TOKEN".to_string(), false),
        ]
    );

    // Its credential was deleted outside Dots.
    secrets.inner.delete("personal/GITHUB_TOKEN").unwrap();
    let listed = vault.list().await.unwrap();
    assert_eq!(
        missing(&listed),
        [
            ("GITHUB_TOKEN".to_string(), true),
            ("LOG_LEVEL".to_string(), false),
            ("NPM_TOKEN".to_string(), false),
        ]
    );
    assert_eq!(
        secrets.gets.load(Ordering::SeqCst),
        gets,
        "a listing reads no value"
    );

    // Setting it again (Replace value) brings it back, and the answer says so.
    let set = vault
        .set_value("GITHUB_TOKEN", PERSONAL_TOKEN)
        .await
        .unwrap();
    assert!(!set.value_missing);
    assert!(!vault.list().await.unwrap()[0].value_missing);
}

#[tokio::test]
async fn no_listing_holds_a_value() {
    let h = OrgsHarness::new().await;
    let vault = h.vault();
    h.join_acme(
        json!({"API_KEY": ORG_KEY, "API_BASE_URL": ORG_URL}),
        json!({"plain": ["API_BASE_URL"]}),
    )
    .await;
    vault
        .add("GITHUB_TOKEN", EntryKind::Secret, PERSONAL_TOKEN)
        .await
        .unwrap();
    h.store
        .create_dot(
            &granting("acme-bot", &[("shell", &["API_KEY"])]),
            Some(ACME),
        )
        .await
        .unwrap();
    h.store
        .create_dot(&granting("mine", &[("shell", &["GITHUB_TOKEN"])]), None)
        .await
        .unwrap();
    h.secrets.delete("personal/GITHUB_TOKEN").unwrap();

    let personal = serde_json::to_string(&vault.list().await.unwrap()).unwrap();
    let org = serde_json::to_string(&h.orgs.entries(ACME).await.unwrap()).unwrap();
    for text in [&personal, &org] {
        for value in [PERSONAL_TOKEN, ORG_KEY, TOKEN] {
            assert!(!text.contains(value), "{text} holds {value:?}");
        }
    }
    // The plain value is the one value an org listing shows.
    assert!(org.contains(ORG_URL), "{org}");
}

#[tokio::test]
async fn an_orgs_used_by_lists_only_the_dots_of_that_org() {
    let h = OrgsHarness::new().await;
    h.join_acme(
        json!({"API_KEY": ORG_KEY, "API_BASE_URL": ORG_URL}),
        json!({"plain": ["API_BASE_URL"]}),
    )
    .await;
    let bot = h
        .store
        .create_dot(
            &granting("acme-bot", &[("shell", &["api_key", "API_BASE_URL"])]),
            Some(ACME),
        )
        .await
        .unwrap();
    let docs = h
        .store
        .create_dot(
            &granting("acme-docs", &[("shell", &["API_BASE_URL"])]),
            Some(ACME),
        )
        .await
        .unwrap();
    // A Personal dot and another org's dot grant the same names.
    h.store
        .create_dot(&granting("mine", &[("shell", &["API_KEY"])]), None)
        .await
        .unwrap();
    h.store
        .create_dot(
            &granting("beta-bot", &[("shell", &["API_KEY"])]),
            Some("beta"),
        )
        .await
        .unwrap();

    let entries = h.orgs.entries(ACME).await.unwrap();
    let used: Vec<(&str, Vec<DotRef>)> = entries
        .iter()
        .map(|e| (e.name.as_str(), e.used_by.clone()))
        .collect();
    assert_eq!(
        used,
        [
            ("API_BASE_URL", vec![named(&bot), named(&docs)]),
            ("API_KEY", vec![named(&bot)]),
        ]
    );
}
