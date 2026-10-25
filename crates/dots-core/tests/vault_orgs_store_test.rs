//! The orgs joined on this PC, and the org each dot belongs to (env vault spec §5.2 and §6.4).

mod common;

use common::{spec, temp_store};
use dots_core::model::Dot;
use dots_core::store::Store;
use dots_core::vault::org::{
    BaoAuth, InfisicalSettings, OpenBaoSettings, OrgRow, SopsSettings, SourceSettings,
};
use dots_core::{Config, Error, Runtime};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePool;
use tempfile::TempDir;

const KEY: &str = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";
const JOINED: &str = "2026-10-22T09:00:00.000Z";

fn openbao() -> SourceSettings {
    SourceSettings::OpenBao(OpenBaoSettings {
        address: "https://bao.acme.example".into(),
        mount: "kv".into(),
        path: "teams/eng".into(),
        auth: BaoAuth::Userpass,
        auth_mount: Some("people".into()),
        namespace: Some("eng".into()),
    })
}

fn infisical() -> SourceSettings {
    SourceSettings::Infisical(InfisicalSettings {
        site_url: "https://app.infisical.com".into(),
        project_id: "6f1c0d7e".into(),
        environment: "prod".into(),
        secret_path: "/dots".into(),
    })
}

fn sops() -> SourceSettings {
    SourceSettings::Sops(SopsSettings {
        location: "https://files.acme.example/dots/env.sops.json".into(),
    })
}

fn org(id: &str, name: &str, source: SourceSettings) -> OrgRow {
    OrgRow {
        id: id.into(),
        name: name.into(),
        source,
        policy_key: KEY.into(),
        last_issued_at: None,
        last_fetch_at: None,
        joined_at: JOINED.into(),
    }
}

fn acme() -> OrgRow {
    org("acme-eng", "Acme Engineering", openbao())
}

async fn raw_pool(dir: &TempDir) -> SqlitePool {
    SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path().join("dots.db").display()
    ))
    .await
    .unwrap()
}

/// The `source` column of the org `id`, as stored.
async fn stored_source(dir: &TempDir, id: &str) -> Value {
    let pool = raw_pool(dir).await;
    let text: String = sqlx::query_scalar("SELECT source FROM orgs WHERE id = ?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    pool.close().await;
    serde_json::from_str(&text).unwrap()
}

fn names(orgs: &[OrgRow]) -> Vec<&str> {
    orgs.iter().map(|o| o.name.as_str()).collect()
}

#[tokio::test]
async fn an_org_of_each_kind_round_trips_with_its_source_as_json() {
    let (dir, store) = temp_store().await;
    assert_eq!(store.get_org("acme-eng").await.unwrap(), None);
    let defaults = SourceSettings::OpenBao(OpenBaoSettings {
        address: "https://bao.acme.example".into(),
        mount: "secret".into(),
        path: "ops".into(),
        auth: BaoAuth::Token,
        auth_mount: None,
        namespace: None,
    });
    let cases = [
        (
            org("acme-eng", "Acme Engineering", openbao()),
            json!({"kind": "openbao", "address": "https://bao.acme.example", "mount": "kv",
                   "path": "teams/eng", "auth": "userpass", "auth_mount": "people",
                   "namespace": "eng"}),
        ),
        (
            org("acme-ops", "Acme Ops", defaults),
            json!({"kind": "openbao", "address": "https://bao.acme.example", "mount": "secret",
                   "path": "ops", "auth": "token"}),
        ),
        (
            org("beta", "Beta", infisical()),
            json!({"kind": "infisical", "site_url": "https://app.infisical.com",
                   "project_id": "6f1c0d7e", "environment": "prod", "secret_path": "/dots"}),
        ),
        (
            org("gamma-1", "Gamma", sops()),
            json!({"kind": "sops",
                   "location": "https://files.acme.example/dots/env.sops.json"}),
        ),
    ];
    for (row, json) in &cases {
        let mut row = row.clone();
        row.last_issued_at = Some("2026-10-22T08:00:00Z".into());
        row.last_fetch_at = Some("2026-10-22T09:00:05.000Z".into());
        store.add_org(&row).await.unwrap();
        assert_eq!(store.get_org(&row.id).await.unwrap().as_ref(), Some(&row));
        assert_eq!(&stored_source(&dir, &row.id).await, json, "{}", row.id);
    }
    assert_eq!(store.list_orgs().await.unwrap().len(), 4);
}

#[tokio::test]
async fn an_org_id_already_joined_is_refused_and_the_first_row_stays() {
    let (_dir, store) = temp_store().await;
    store.add_org(&acme()).await.unwrap();
    let again = org("acme-eng", "Acme Again", sops());
    let err = store.add_org(&again).await.unwrap_err();
    let want = "the org 'acme-eng' is already joined on this PC";
    assert!(matches!(&err, Error::Conflict(m) if m == want), "{err:?}");
    assert_eq!(store.get_org("acme-eng").await.unwrap(), Some(acme()));
}

#[tokio::test]
async fn an_org_id_that_breaks_the_rule_is_refused() {
    let (_dir, store) = temp_store().await;
    for id in ["", "Acme", "acme_eng", "-acme", "../acme"] {
        let err = store
            .add_org(&org(id, "Acme", openbao()))
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{id}: {err:?}");
    }
    assert!(store.list_orgs().await.unwrap().is_empty());
}

#[tokio::test]
async fn orgs_are_listed_by_name_without_regard_to_case() {
    let (_dir, store) = temp_store().await;
    assert!(store.list_orgs().await.unwrap().is_empty());
    for (id, name) in [
        ("z", "Zeta"),
        ("a2", "acme Design"),
        ("b", "beta"),
        ("a1", "Acme Engineering"),
        ("a0", "Acme Engineering"),
    ] {
        store.add_org(&org(id, name, sops())).await.unwrap();
    }
    let listed = store.list_orgs().await.unwrap();
    assert_eq!(
        names(&listed),
        [
            "acme Design",
            "Acme Engineering",
            "Acme Engineering",
            "beta",
            "Zeta"
        ]
    );
    // Two orgs of one name come by id.
    assert_eq!(listed[1].id, "a0");
    assert_eq!(listed[2].id, "a1");
}

#[tokio::test]
async fn updating_the_settings_keeps_what_the_last_fetch_accepted() {
    let (dir, store) = temp_store().await;
    store.add_org(&acme()).await.unwrap();
    store
        .record_org_fetch(
            "acme-eng",
            "2026-10-22T09:00:05.000Z",
            "2026-10-22T08:00:00Z",
        )
        .await
        .unwrap();
    let new_key = "RWTdnu2/6v8wB3XtPmO0VDUhd2DcHn1D5yN0FTcpUOFnnC6IbYTEQQts";
    store
        .update_org_settings("acme-eng", "Acme Eng", &infisical(), new_key)
        .await
        .unwrap();
    let want = OrgRow {
        name: "Acme Eng".into(),
        source: infisical(),
        policy_key: new_key.into(),
        last_issued_at: Some("2026-10-22T08:00:00Z".into()),
        last_fetch_at: Some("2026-10-22T09:00:05.000Z".into()),
        ..acme()
    };
    assert_eq!(store.get_org("acme-eng").await.unwrap(), Some(want));
    assert_eq!(stored_source(&dir, "acme-eng").await["kind"], "infisical");

    let err = store
        .update_org_settings("nope", "Nope", &sops(), KEY)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::NotFound(m) if m == "no org 'nope' is joined on this PC"),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_good_fetch_is_recorded_and_the_next_one_replaces_it() {
    let (_dir, store) = temp_store().await;
    store.add_org(&acme()).await.unwrap();
    store.add_org(&org("beta", "Beta", sops())).await.unwrap();
    store
        .record_org_fetch(
            "acme-eng",
            "2026-10-22T09:00:05.000Z",
            "2026-10-22T08:00:00Z",
        )
        .await
        .unwrap();
    let got = store.get_org("acme-eng").await.unwrap().unwrap();
    assert_eq!(
        got.last_fetch_at.as_deref(),
        Some("2026-10-22T09:00:05.000Z")
    );
    assert_eq!(got.last_issued_at.as_deref(), Some("2026-10-22T08:00:00Z"));
    assert_eq!(got.joined_at, JOINED);

    store
        .record_org_fetch(
            "acme-eng",
            "2026-10-22T09:15:05.000Z",
            "2026-10-22T09:10:00Z",
        )
        .await
        .unwrap();
    let got = store.get_org("acme-eng").await.unwrap().unwrap();
    assert_eq!(
        got.last_fetch_at.as_deref(),
        Some("2026-10-22T09:15:05.000Z")
    );
    assert_eq!(got.last_issued_at.as_deref(), Some("2026-10-22T09:10:00Z"));
    // Another org's row is untouched.
    let beta = store.get_org("beta").await.unwrap().unwrap();
    assert_eq!((beta.last_fetch_at, beta.last_issued_at), (None, None));

    let err = store
        .record_org_fetch("nope", "2026-10-22T09:15:05.000Z", "2026-10-22T09:10:00Z")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn a_removed_orgs_dots_keep_its_id_and_a_re_join_attaches_them_again() {
    let (_dir, store) = temp_store().await;
    store.add_org(&acme()).await.unwrap();
    let dot = store
        .create_dot(&spec("deploy"), Some("acme-eng"))
        .await
        .unwrap();
    assert_eq!(dot.org_id.as_deref(), Some("acme-eng"));

    store.remove_org("acme-eng").await.unwrap();
    assert_eq!(store.get_org("acme-eng").await.unwrap(), None);
    assert!(store.list_orgs().await.unwrap().is_empty());
    assert_eq!(store.get_dot(&dot.id).await.unwrap(), dot);
    let err = store.remove_org("acme-eng").await.unwrap_err();
    assert!(
        matches!(&err, Error::NotFound(m) if m == "no org 'acme-eng' is joined on this PC"),
        "{err:?}"
    );

    // Joining it again finds the dot where it was: the column has no foreign key.
    store.add_org(&acme()).await.unwrap();
    let org_id = store.get_dot(&dot.id).await.unwrap().org_id.unwrap();
    assert_eq!(store.get_org(&org_id).await.unwrap(), Some(acme()));
}

#[tokio::test]
async fn a_stored_source_that_no_longer_reads_is_an_error_naming_the_org() {
    let (dir, store) = temp_store().await;
    store.add_org(&acme()).await.unwrap();
    store.add_org(&org("beta", "Beta", sops())).await.unwrap();
    let pool = raw_pool(&dir).await;
    sqlx::query("UPDATE orgs SET source = ? WHERE id = 'acme-eng'")
        .bind(json!({"kind": "dropbox", "folder": "x"}).to_string())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let err = store.get_org("acme-eng").await.unwrap_err();
    let text = err.to_string();
    assert!(
        text.starts_with("org 'acme-eng': its stored source settings are not readable: "),
        "{text}"
    );
    assert!(text.contains("dropbox"), "{text}");
    assert!(store.list_orgs().await.is_err());
    // The other org still reads.
    assert!(store.get_org("beta").await.unwrap().is_some());
}

async fn runtime() -> (TempDir, Runtime) {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    // A missing program: no test may run a `claude` found on the PATH.
    cfg.claude_path = Some(dir.path().join("no-claude.exe"));
    let rt = Runtime::start(cfg).await.unwrap();
    (dir, rt)
}

#[tokio::test]
async fn the_runtime_refuses_a_dot_in_an_org_that_is_not_joined() {
    let (_dir, rt) = runtime().await;
    let err = rt
        .create_dot(spec("deploy"), Some("acme-eng".into()))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::Invalid(m) if m == "no org 'acme-eng' is joined on this PC"),
        "{err:?}"
    );
    assert!(rt.list_dots().await.unwrap().is_empty());
    rt.shutdown();
}

#[tokio::test]
async fn the_runtime_stores_the_org_of_a_dot_in_a_joined_org() {
    let (_dir, rt) = runtime().await;
    rt.store().add_org(&acme()).await.unwrap();
    let dot = rt
        .create_dot(spec("deploy"), Some("acme-eng".into()))
        .await
        .unwrap();
    assert_eq!(dot.org_id.as_deref(), Some("acme-eng"));
    assert_eq!(rt.get_dot(&dot.id).await.unwrap(), dot);
    let personal = rt.create_dot(spec("notes"), None).await.unwrap();
    assert_eq!(personal.org_id, None);
    rt.shutdown();
}

#[tokio::test]
async fn a_dot_shows_its_org_as_org_id() {
    let (_dir, store) = temp_store().await;
    let in_org = store
        .create_dot(&spec("deploy"), Some("acme-eng"))
        .await
        .unwrap();
    let personal = store.create_dot(&spec("notes"), None).await.unwrap();
    assert_eq!(serde_json::to_value(&in_org).unwrap()["org_id"], "acme-eng");
    assert_eq!(
        serde_json::to_value(&personal).unwrap()["org_id"],
        Value::Null
    );
    // A dot written without the field, as before plan B, reads as Personal.
    let mut old = serde_json::to_value(&personal).unwrap();
    old.as_object_mut().unwrap().remove("org_id");
    let read: Dot = serde_json::from_value(old).unwrap();
    assert_eq!(read, personal);
}

#[tokio::test]
async fn the_orgs_table_survives_a_reopen() {
    let (dir, store) = temp_store().await;
    store.add_org(&acme()).await.unwrap();
    store.close().await;
    let store = Store::open(&dir.path().join("dots.db")).await.unwrap();
    assert_eq!(store.list_orgs().await.unwrap(), [acme()]);
}
