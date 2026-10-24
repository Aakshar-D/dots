//! MCP catalogs kept per scope (env vault spec §8): a server's tool list accepted with
//! Personal's values and one accepted with an org's are kept apart, a change to the server
//! resets both and a delete removes both. The Library's Check runs in a chosen scope and, for an
//! org, resolves the server's variables as a run in that org would. A run reads the catalog of
//! its dot's scope. Migration 0013 keeps every catalog made before it as Personal's.

mod common;

use std::collections::BTreeMap;
use std::time::Duration as StdDuration;

use chrono::Duration;
use common::fake_mcp::{tool, FakeMcp};
use common::fake_openbao::Failure;
use common::orgs::{OrgsHarness, ACME, TOKEN};
use dots_core::events::new_bus;
use dots_core::library::{
    CatalogStatus, LibraryConfig, LibraryItem, McpCatalog, Uses, PERSONAL_SCOPE,
};
use dots_core::library_service::LibraryService;
use dots_core::mcp::fingerprint;
use dots_core::run_tools::{OpenRequest, RunTools};
use dots_core::store::Store;
use dots_core::vault::{DotScope, EntryKind, Grants, Vault};
use dots_core::Error;
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool};
use tokio_util::sync::CancellationToken;

const ORG_KEY: &str = "org-secret-api-key-71c0d2";
const ORG_PROD: &str = "org-secret-prod-db-4e9a17";
const PERSONAL_KEY: &str = "personal-secret-api-key-0b6f3e";
const PERSONAL_ONLY: &str = "personal-secret-only-c8d215";

/// Every value a test serves or stores, which no stored error may hold.
const VALUES: &[&str] = &[ORG_KEY, ORG_PROD, PERSONAL_KEY, PERSONAL_ONLY, TOKEN];

fn assert_clean(text: &str) {
    for value in VALUES {
        assert!(!text.contains(value), "{text:?} holds {value:?}");
    }
}

fn acme() -> DotScope {
    DotScope::Org(ACME.into())
}

/// A catalog in which `fp` was seen and is waiting for review.
fn seen(server_id: &str, fp: &str) -> McpCatalog {
    McpCatalog {
        seen_fingerprint: Some(fp.into()),
        seen_tools: Some(json!([{"name": fp}])),
        status: CatalogStatus::Changed,
        checked_at: Some("2026-10-22T09:00:00.000Z".into()),
        ..McpCatalog::unchecked(server_id)
    }
}

fn stdio(command: &str) -> LibraryConfig {
    LibraryConfig::McpServer(
        serde_json::from_value(json!({"transport": "stdio", "command": command})).unwrap(),
    )
}

async fn add_server(store: &Store, name: &str) -> LibraryItem {
    store
        .add_library_item(name, &stdio("sf-mcp"), None)
        .await
        .unwrap()
}

/// The catalogs stored for `server_id`, by scope.
async fn scopes_of(store: &Store, server_id: &str) -> Vec<String> {
    store
        .list_catalogs()
        .await
        .unwrap()
        .into_iter()
        .filter(|(_, c)| c.server_id == server_id)
        .map(|(scope, _)| scope)
        .collect()
}

// Migration 0013

/// A database Dots left at 0012, with a server whose list was accepted and one never checked:
/// both catalogs come out as Personal's with every field, and a second scope can be added.
#[tokio::test]
async fn migration_0013_keeps_every_catalog_as_personal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dots.db");
    let opts = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
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
    for (id, name) in [("S1", "sf"), ("S2", "gh")] {
        sqlx::query(
            "INSERT INTO library_items (id, kind, name, config, owner_id, created_at, \
             updated_at) VALUES (?, 'mcp_server', ?, ?, NULL, '2026-10-01T00:00:00.000Z', \
             '2026-10-01T00:00:00.000Z')",
        )
        .bind(id)
        .bind(name)
        .bind(json!({"transport": "stdio", "command": "sf-mcp"}).to_string())
        .execute(&pool)
        .await
        .unwrap();
    }
    let tools = json!([{"name": "query", "annotations": {"readOnlyHint": true}}]);
    sqlx::query(
        "INSERT INTO mcp_catalogs (server_id, accepted_fingerprint, accepted_tools, \
         seen_fingerprint, seen_tools, status, checked_at, error) \
         VALUES ('S1', 'f1', ?, 'f2', '[]', 'unreachable', '2026-10-02T00:00:00.000Z', \
         'could not start: timed out')",
    )
    .bind(tools.to_string())
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO mcp_catalogs (server_id, status) VALUES ('S2', 'unchecked')")
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let store = Store::open(&path).await.unwrap();
    let s1 = McpCatalog {
        server_id: "S1".into(),
        status: CatalogStatus::Unreachable,
        accepted_fingerprint: Some("f1".into()),
        accepted_tools: Some(tools),
        seen_fingerprint: Some("f2".into()),
        seen_tools: Some(json!([])),
        checked_at: Some("2026-10-02T00:00:00.000Z".into()),
        error: Some("could not start: timed out".into()),
    };
    assert_eq!(
        store.list_catalogs().await.unwrap(),
        [
            (PERSONAL_SCOPE.to_string(), s1.clone()),
            (PERSONAL_SCOPE.to_string(), McpCatalog::unchecked("S2")),
        ]
    );
    assert_eq!(store.get_catalog("S1", PERSONAL_SCOPE).await.unwrap(), s1);

    // The key is the server and the scope; the server's delete still takes every scope's.
    store.put_catalog(ACME, &seen("S1", "f9")).await.unwrap();
    assert_eq!(scopes_of(&store, "S1").await, [ACME, PERSONAL_SCOPE]);
    assert_eq!(store.get_catalog("S1", PERSONAL_SCOPE).await.unwrap(), s1);
    store.remove_library_item("S1").await.unwrap();
    assert!(scopes_of(&store, "S1").await.is_empty());

    let pool = SqlitePool::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    let key: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM pragma_table_info('mcp_catalogs') WHERE pk > 0 ORDER BY pk",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(key, ["server_id", "scope"]);
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type = 'table' AND name LIKE 'mcp_catalogs%'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(tables, ["mcp_catalogs"]);
    pool.close().await;
}

// The store

#[tokio::test]
async fn a_new_server_has_an_unchecked_personal_catalog_and_none_stored_in_an_org() {
    let h = OrgsHarness::new().await;
    let sf = add_server(&h.store, "sf").await;
    assert_eq!(scopes_of(&h.store, &sf.id).await, [PERSONAL_SCOPE]);
    let unchecked = McpCatalog::unchecked(&sf.id);
    assert_eq!(
        h.store.get_catalog(&sf.id, PERSONAL_SCOPE).await.unwrap(),
        unchecked
    );
    // Never checked in the org: it reads as unchecked, and nothing is stored for it.
    assert_eq!(h.store.get_catalog(&sf.id, ACME).await.unwrap(), unchecked);
    assert_eq!(scopes_of(&h.store, &sf.id).await, [PERSONAL_SCOPE]);
    // Nothing seen in the org: nothing to accept there.
    let err = h
        .store
        .accept_catalog(&sf.id, ACME, "f1")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");

    let err = h.store.get_catalog("nope", ACME).await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    let err = h
        .store
        .record_catalog_seen("nope", ACME, "f1", &json!([]))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    for bad in ["", "Personal", "../x", "acme eng"] {
        let err = h.store.get_catalog(&sf.id, bad).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{bad:?}: {err:?}");
    }
}

#[tokio::test]
async fn the_first_record_in_an_org_makes_its_catalog() {
    let h = OrgsHarness::new().await;
    let sf = add_server(&h.store, "sf").await;
    let tools = json!([{"name": "query"}]);
    let c = h
        .store
        .record_catalog_seen(&sf.id, ACME, "f1", &tools)
        .await
        .unwrap();
    assert_eq!(c.status, CatalogStatus::Changed);
    assert_eq!(c.seen_tools, Some(tools));
    assert_eq!(scopes_of(&h.store, &sf.id).await, [ACME, PERSONAL_SCOPE]);

    let gh = add_server(&h.store, "gh").await;
    let c = h
        .store
        .record_catalog_failure(&gh.id, ACME, "API_KEY: not in Acme Engineering")
        .await
        .unwrap();
    assert_eq!(c.status, CatalogStatus::Unreachable);
    assert_eq!(c.error.as_deref(), Some("API_KEY: not in Acme Engineering"));
    assert_eq!(scopes_of(&h.store, &gh.id).await, [ACME, PERSONAL_SCOPE]);
    assert_eq!(
        h.store.get_catalog(&gh.id, PERSONAL_SCOPE).await.unwrap(),
        McpCatalog::unchecked(&gh.id)
    );
}

#[tokio::test]
async fn two_scopes_of_one_server_are_kept_apart() {
    let h = OrgsHarness::new().await;
    let sf = add_server(&h.store, "sf").await;

    // Accepted in Personal; the org has not seen it.
    h.store
        .record_catalog_seen(&sf.id, PERSONAL_SCOPE, "f1", &json!([]))
        .await
        .unwrap();
    let accepted = h
        .store
        .accept_catalog(&sf.id, PERSONAL_SCOPE, "f1")
        .await
        .unwrap();
    assert_eq!(accepted.status, CatalogStatus::Accepted);
    assert_eq!(
        h.store.get_catalog(&sf.id, ACME).await.unwrap(),
        McpCatalog::unchecked(&sf.id)
    );
    // The same list seen in the org is not accepted there: Personal's accept is Personal's.
    let in_org = h
        .store
        .record_catalog_seen(&sf.id, ACME, "f1", &json!([]))
        .await
        .unwrap();
    assert_eq!(in_org.status, CatalogStatus::Changed);
    assert_eq!(in_org.accepted_fingerprint, None);
    assert_eq!(
        h.store.changed_mcp_servers(ACME).await.unwrap(),
        std::slice::from_ref(&sf)
    );
    assert!(h
        .store
        .changed_mcp_servers(PERSONAL_SCOPE)
        .await
        .unwrap()
        .is_empty());

    // The other way: accepted in the org, while Personal sees another list.
    let gh = add_server(&h.store, "gh").await;
    h.store
        .put_catalog(ACME, &seen(&gh.id, "g1"))
        .await
        .unwrap();
    h.store.accept_catalog(&gh.id, ACME, "g1").await.unwrap();
    let personal = h
        .store
        .record_catalog_seen(&gh.id, PERSONAL_SCOPE, "g2", &json!([]))
        .await
        .unwrap();
    assert_eq!(personal.status, CatalogStatus::Changed);
    assert_eq!(personal.accepted_fingerprint, None);
    let err = h
        .store
        .accept_catalog(&gh.id, PERSONAL_SCOPE, "g1")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    let org = h.store.get_catalog(&gh.id, ACME).await.unwrap();
    assert_eq!(org.status, CatalogStatus::Accepted);
    assert_eq!(org.accepted_fingerprint.as_deref(), Some("g1"));
    // A failure in one scope leaves the other.
    h.store
        .record_catalog_failure(&gh.id, PERSONAL_SCOPE, "could not start")
        .await
        .unwrap();
    assert_eq!(h.store.get_catalog(&gh.id, ACME).await.unwrap(), org);
}

#[tokio::test]
async fn changing_a_server_resets_its_catalog_in_every_scope() {
    let h = OrgsHarness::new().await;
    let sf = add_server(&h.store, "sf").await;
    for (scope, fp) in [(PERSONAL_SCOPE, "p1"), (ACME, "o1")] {
        h.store.put_catalog(scope, &seen(&sf.id, fp)).await.unwrap();
        h.store.accept_catalog(&sf.id, scope, fp).await.unwrap();
    }
    h.store
        .update_library_item(&sf.id, &stdio("sf-mcp-2"))
        .await
        .unwrap();
    for (scope, fp) in [(PERSONAL_SCOPE, "p1"), (ACME, "o1")] {
        let c = h.store.get_catalog(&sf.id, scope).await.unwrap();
        assert_eq!(c.status, CatalogStatus::Unchecked, "{scope}");
        assert_eq!(c.seen_fingerprint, None, "{scope}");
        assert_eq!(c.checked_at, None, "{scope}");
        // The accepted list stays for the review, in each scope.
        assert_eq!(c.accepted_fingerprint.as_deref(), Some(fp), "{scope}");
    }

    // An http server that moves to another origin keeps nothing accepted, in any scope.
    let http = |url: &str| {
        LibraryConfig::McpServer(
            serde_json::from_value(json!({"transport": "http", "url": url})).unwrap(),
        )
    };
    let gh = h
        .store
        .add_library_item("gh", &http("https://a.example.invalid/mcp"), None)
        .await
        .unwrap();
    for scope in [PERSONAL_SCOPE, ACME] {
        h.store
            .put_catalog(scope, &seen(&gh.id, "h1"))
            .await
            .unwrap();
        h.store.accept_catalog(&gh.id, scope, "h1").await.unwrap();
    }
    h.store
        .update_library_item(&gh.id, &http("https://b.example.invalid/mcp"))
        .await
        .unwrap();
    for scope in [PERSONAL_SCOPE, ACME] {
        let c = h.store.get_catalog(&gh.id, scope).await.unwrap();
        assert_eq!(c, McpCatalog::unchecked(&gh.id), "{scope}");
    }
}

#[tokio::test]
async fn deleting_a_server_deletes_its_catalog_in_every_scope() {
    let h = OrgsHarness::new().await;
    let sf = add_server(&h.store, "sf").await;
    let gh = add_server(&h.store, "gh").await;
    h.store
        .put_catalog(ACME, &seen(&sf.id, "o1"))
        .await
        .unwrap();
    h.store
        .put_catalog("beta", &seen(&sf.id, "b1"))
        .await
        .unwrap();
    assert_eq!(
        scopes_of(&h.store, &sf.id).await,
        [ACME, "beta", PERSONAL_SCOPE]
    );

    h.store.remove_library_item(&sf.id).await.unwrap();
    assert!(scopes_of(&h.store, &sf.id).await.is_empty());
    for scope in [PERSONAL_SCOPE, ACME, "beta"] {
        let err = h.store.get_catalog(&sf.id, scope).await.unwrap_err();
        assert!(matches!(err, Error::NotFound(_)), "{scope}: {err:?}");
    }
    let err = h
        .store
        .record_catalog_failure(&sf.id, ACME, "gone")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
    assert_eq!(scopes_of(&h.store, &gh.id).await, [PERSONAL_SCOPE]);
}

// The Library's Check in a scope

/// Acme joined with `API_KEY` and `PROD_DB`, a policy of `extra`, and Personal holding
/// `API_KEY` and `IN_PERSONAL`; the Library on that vault.
async fn setup(extra: Value) -> (OrgsHarness, Vault, LibraryService) {
    let h = OrgsHarness::new().await;
    h.join_acme(json!({"API_KEY": ORG_KEY, "PROD_DB": ORG_PROD}), extra)
        .await;
    let vault = h.vault();
    vault
        .add("API_KEY", EntryKind::Secret, PERSONAL_KEY)
        .await
        .unwrap();
    vault
        .add("IN_PERSONAL", EntryKind::Secret, PERSONAL_ONLY)
        .await
        .unwrap();
    let lib = LibraryService::new(h.store.clone(), new_bus()).with_vault(vault.clone());
    (h, vault, lib)
}

/// A `fake-mcp` server named `name` that reads `${var}` into its `FAKE_MCP_VALUE`.
async fn server_reading(store: &Store, name: &str, var: &str) -> (FakeMcp, LibraryItem) {
    let fake = FakeMcp::new(json!({"tools": [tool("query", true)]}));
    let env = BTreeMap::from([("FAKE_MCP_VALUE".to_string(), format!("${{{var}}}"))]);
    let config = LibraryConfig::McpServer(fake.stdio_with(Vec::new(), env));
    let item = store.add_library_item(name, &config, None).await.unwrap();
    (fake, item)
}

/// The `FAKE_MCP_VALUE` each start of the server had.
fn started_with(fake: &FakeMcp) -> Vec<String> {
    fake.log()
        .into_iter()
        .filter_map(|r| r.get("start").cloned())
        .map(|s| {
            s["env"]["FAKE_MCP_VALUE"]
                .as_str()
                .unwrap_or("")
                .to_string()
        })
        .collect()
}

#[tokio::test]
async fn a_check_in_an_org_reads_the_orgs_value_into_the_orgs_catalog() {
    let (h, _vault, lib) = setup(json!({"personal": "deny"})).await;
    let (fake, item) = server_reading(&h.store, "sf", "API_KEY").await;

    let c = lib.check(&item.id, &acme()).await.unwrap();
    assert_eq!(c.status, CatalogStatus::Changed, "{c:?}");
    assert_eq!(started_with(&fake), [ORG_KEY]);
    let in_org = h.store.get_catalog(&item.id, ACME).await.unwrap();
    assert_eq!(in_org, c);
    let fp = in_org.seen_fingerprint.clone().unwrap();
    assert_eq!(
        h.store.get_catalog(&item.id, PERSONAL_SCOPE).await.unwrap(),
        McpCatalog::unchecked(&item.id),
        "Personal's catalog is not touched"
    );

    let accepted = lib.accept(&item.id, &acme(), &fp).await.unwrap();
    assert_eq!(accepted.status, CatalogStatus::Accepted);
    let c = lib.check(&item.id, &DotScope::Personal).await.unwrap();
    assert_eq!(c.status, CatalogStatus::Changed);
    assert_eq!(started_with(&fake), [ORG_KEY, PERSONAL_KEY]);
    assert_eq!(h.store.get_catalog(&item.id, ACME).await.unwrap(), accepted);
    let tools = c.seen_tools.clone().unwrap();
    assert_eq!(
        fingerprint(&serde_json::from_value::<Vec<_>>(tools).unwrap()),
        fp,
        "the same list, in both scopes"
    );
}

#[tokio::test]
async fn a_check_in_an_org_under_fallback_reads_a_personal_value_the_org_lacks() {
    let (h, _vault, lib) = setup(json!({"personal": "fallback"})).await;
    let (fake, item) = server_reading(&h.store, "sf", "IN_PERSONAL").await;
    let c = lib.check(&item.id, &acme()).await.unwrap();
    assert_eq!(c.status, CatalogStatus::Changed, "{c:?}");
    assert_eq!(started_with(&fake), [PERSONAL_ONLY]);

    // Under fallback the org's own entry still wins.
    let (fake, item) = server_reading(&h.store, "gh", "API_KEY").await;
    lib.check(&item.id, &acme()).await.unwrap();
    assert_eq!(started_with(&fake), [ORG_KEY]);
}

#[tokio::test]
async fn a_check_in_an_org_that_lacks_a_variable_names_the_scope_and_the_variable() {
    let (h, _vault, lib) = setup(json!({"personal": "deny"})).await;
    let (fake, item) = server_reading(&h.store, "sf", "IN_PERSONAL").await;
    let c = lib.check(&item.id, &acme()).await.unwrap();
    assert_eq!(c.status, CatalogStatus::Unreachable);
    assert_eq!(
        c.error.as_deref(),
        Some("IN_PERSONAL: not in Acme Engineering")
    );
    assert!(fake.log().is_empty(), "the server was not started");
    assert_eq!(h.store.get_catalog(&item.id, ACME).await.unwrap(), c);
    assert_eq!(
        h.store.get_catalog(&item.id, PERSONAL_SCOPE).await.unwrap(),
        McpCatalog::unchecked(&item.id)
    );

    // In Personal the same server starts with Personal's value.
    let c = lib.check(&item.id, &DotScope::Personal).await.unwrap();
    assert_eq!(c.status, CatalogStatus::Changed);
    assert_eq!(started_with(&fake), [PERSONAL_ONLY]);
}

#[tokio::test]
async fn a_check_of_a_name_the_orgs_use_rules_deny_fails_without_a_value() {
    let (h, _vault, lib) =
        setup(json!({"personal": "override", "use": {"deny": ["PROD_*"]}})).await;
    let (fake, item) = server_reading(&h.store, "db", "PROD_DB").await;
    let c = lib.check(&item.id, &acme()).await.unwrap();
    assert_eq!(c.status, CatalogStatus::Unreachable);
    let error = c.error.clone().unwrap();
    assert_eq!(
        error,
        "PROD_DB: Acme Engineering's policy does not allow it"
    );
    assert_clean(&error);
    assert_clean(&format!("{c:?}"));
    assert!(fake.log().is_empty(), "the server was not started");
    assert!(!error.contains("dot"), "a check names no dot: {error}");
}

#[tokio::test]
async fn a_check_in_an_expired_or_revoked_org_fails_with_its_reason() {
    let (h, _vault, lib) = setup(json!({"personal": "override", "offline_hours": 2})).await;
    let (fake, item) = server_reading(&h.store, "sf", "API_KEY").await;
    h.bao.fail_next(Failure::Status(503), 1);
    h.orgs.refresh(ACME).await.unwrap();
    h.advance(Duration::hours(2));
    let c = lib.check(&item.id, &acme()).await.unwrap();
    assert_eq!(c.status, CatalogStatus::Unreachable);
    assert_eq!(
        c.error.as_deref(),
        Some("API_KEY: Acme Engineering: secrets expired offline; reconnect to refresh")
    );

    // Reconnected, then revoked.
    h.orgs.refresh(ACME).await.unwrap();
    h.bao.revoke_token(TOKEN);
    h.orgs.refresh(ACME).await.unwrap();
    let c = lib.check(&item.id, &acme()).await.unwrap();
    assert_eq!(
        c.error.as_deref(),
        Some("API_KEY: Acme Engineering: access revoked")
    );
    assert_clean(&format!("{c:?}"));
    assert!(fake.log().is_empty(), "the server was never started");
}

#[tokio::test]
async fn a_check_in_an_org_that_is_not_joined_is_refused() {
    let (h, _vault, lib) = setup(json!({})).await;
    let (fake, item) = server_reading(&h.store, "sf", "API_KEY").await;
    let err = lib
        .check(&item.id, &DotScope::Org("beta".into()))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::NotFound(m) if m == "no org 'beta' is joined on this PC"),
        "{err:?}"
    );
    assert_eq!(scopes_of(&h.store, &item.id).await, [PERSONAL_SCOPE]);
    assert!(fake.log().is_empty());

    // A vault with no orgs at all refuses every org.
    let plain = LibraryService::new(h.store.clone(), new_bus())
        .with_vault(Vault::new(h.store.clone(), h.secrets.clone()));
    let err = plain.check(&item.id, &acme()).await.unwrap_err();
    assert!(
        matches!(&err, Error::NotFound(m) if m == "no org 'acme-eng' is joined on this PC"),
        "{err:?}"
    );
}

// A run reads its dot's scope

async fn open(h: &OrgsHarness, vault: &Vault, scope: &DotScope, name: &str) -> RunTools {
    let uses = Uses {
        mcp_servers: vec![name.to_string()],
        ..Default::default()
    };
    let grants = Grants([(format!("mcp:{name}"), vec!["API_KEY".to_string()])].into());
    let (tools, notices) = RunTools::open(OpenRequest {
        store: &h.store,
        bus: &new_bus(),
        run_id: "01RUN",
        uses: &uses,
        workspace: h.data.path(),
        plugins_dir: None,
        cancel: &CancellationToken::new(),
        vault,
        scope,
        grants: &grants,
        dot_name: "triage",
    })
    .await
    .unwrap();
    assert_eq!(tools.servers().len(), 1, "{notices:?}");
    tools
}

#[tokio::test]
async fn a_run_in_an_org_dot_reads_the_orgs_catalog_not_personals() {
    let (h, vault, lib) = setup(json!({"personal": "deny"})).await;
    let (fake, item) = server_reading(&h.store, "sf", "API_KEY").await;

    // Accepted in Personal only: a run in the org has never seen it, so it is held, as an
    // unchecked server is, and its list is recorded in the org's catalog.
    let c = lib.check(&item.id, &DotScope::Personal).await.unwrap();
    let fp = c.seen_fingerprint.unwrap();
    lib.accept(&item.id, &DotScope::Personal, &fp)
        .await
        .unwrap();
    let tools = open(&h, &vault, &acme(), "sf").await;
    assert!(tools.servers()[0].held());
    assert_eq!(tools.servers()[0].accepted_fingerprint(), None);
    tools.close().await;
    let in_org = h.store.get_catalog(&item.id, ACME).await.unwrap();
    assert_eq!(in_org.status, CatalogStatus::Changed);
    assert_eq!(in_org.seen_fingerprint.as_deref(), Some(fp.as_str()));
    assert_eq!(
        h.store
            .get_catalog(&item.id, PERSONAL_SCOPE)
            .await
            .unwrap()
            .status,
        CatalogStatus::Accepted
    );
    // A Personal run of the same server is not held.
    let tools = open(&h, &vault, &DotScope::Personal, "sf").await;
    assert!(!tools.servers()[0].held());
    tools.close().await;

    // Accepted in the org: an org run is not held, and reads the org's value.
    lib.accept(&item.id, &acme(), &fp).await.unwrap();
    let tools = open(&h, &vault, &acme(), "sf").await;
    assert!(!tools.servers()[0].held());
    assert_eq!(
        tools.servers()[0].accepted_fingerprint().as_deref(),
        Some(fp.as_str())
    );
    tools.close().await;
    assert_eq!(
        started_with(&fake),
        [PERSONAL_KEY, ORG_KEY, PERSONAL_KEY, ORG_KEY]
    );
    assert!(fake.gone_within(StdDuration::from_secs(10)).await);
}
