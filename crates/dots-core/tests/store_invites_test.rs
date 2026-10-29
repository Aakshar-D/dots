//! Invites, roster drafts and the pending request of an account (accounts spec §7 and §6.5).

mod common;

use std::collections::BTreeMap;

use common::temp_store;
use dots_core::accounts::{Role, WorkspaceRole};
use dots_core::store::{DraftRow, InviteRow, Store};
use dots_core::vault::org::{
    BaoAuth, MemberRow, OpenBaoSettings, OrgRow, PendingRequest, SourceSettings,
};
use dots_core::Error;

const KEY: &str = "RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3";

fn org(id: &str) -> OrgRow {
    OrgRow {
        id: id.into(),
        name: "Acme Engineering".into(),
        source: SourceSettings::OpenBao(OpenBaoSettings {
            address: "https://bao.acme.example".into(),
            mount: "kv".into(),
            path: "teams/eng".into(),
            auth: BaoAuth::Userpass,
            auth_mount: None,
            namespace: None,
        }),
        policy_key: KEY.into(),
        last_issued_at: None,
        last_fetch_at: None,
        joined_at: "2026-10-22T09:00:00.000Z".into(),
        member: MemberRow::default(),
        pending_request: None,
    }
}

async fn store_with(id: &str) -> (tempfile::TempDir, Store) {
    let (dir, store) = temp_store().await;
    store.add_org(&org(id)).await.unwrap();
    (dir, store)
}

fn invite(id: &str, org_id: &str, created_at: &str) -> InviteRow {
    InviteRow {
        id: id.into(),
        org_id: org_id.into(),
        email: "sam@acme.example".into(),
        role: Role::Member,
        workspaces: BTreeMap::from([("eng".to_string(), WorkspaceRole::Admin)]),
        created_at: created_at.into(),
        expires_at: "2026-11-05T09:00:00.000Z".into(),
        used_at: None,
        revoked_at: None,
    }
}

fn draft(org_id: &str, seq: u64, summary: &str) -> DraftRow {
    DraftRow {
        org_id: org_id.into(),
        base_seq: seq - 1,
        seq,
        signed: format!("signed-{seq}"),
        summary: summary.into(),
        error: String::new(),
        created_at: "2026-10-29T10:00:00.000Z".into(),
        invite_id: None,
    }
}

#[tokio::test]
async fn invites_round_trip_list_newest_first_used_and_revoked() {
    let (_dir, store) = store_with("acme-eng").await;
    let first = invite("inv-1", "acme-eng", "2026-10-29T09:00:00.000Z");
    let second = invite("inv-2", "acme-eng", "2026-10-29T11:00:00.000Z");
    store.add_invite(&first).await.unwrap();
    store.add_invite(&second).await.unwrap();

    assert_eq!(
        store.invite("acme-eng", "inv-1").await.unwrap(),
        Some(first.clone())
    );
    assert_eq!(store.invite("acme-eng", "nope").await.unwrap(), None);
    let listed = store.list_invites("acme-eng").await.unwrap();
    assert_eq!(listed, vec![second.clone(), first.clone()]);

    store
        .mark_invite_used("acme-eng", "inv-1", "2026-10-29T12:00:00.000Z")
        .await
        .unwrap();
    store
        .revoke_invite("acme-eng", "inv-2", "2026-10-29T12:30:00.000Z")
        .await
        .unwrap();
    let got = store.invite("acme-eng", "inv-1").await.unwrap().unwrap();
    assert_eq!(got.used_at.as_deref(), Some("2026-10-29T12:00:00.000Z"));
    assert_eq!(got.revoked_at, None);
    let got = store.invite("acme-eng", "inv-2").await.unwrap().unwrap();
    assert_eq!(got.revoked_at.as_deref(), Some("2026-10-29T12:30:00.000Z"));

    let err = store
        .revoke_invite("acme-eng", "missing", "2026-10-29T12:30:00.000Z")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn put_draft_replaces_the_previous_one() {
    let (_dir, store) = store_with("acme-eng").await;
    assert_eq!(store.draft("acme-eng").await.unwrap(), None);
    store
        .put_draft(&draft("acme-eng", 5, "first"))
        .await
        .unwrap();
    let newer = draft("acme-eng", 6, "second");
    store.put_draft(&newer).await.unwrap();
    assert_eq!(store.draft("acme-eng").await.unwrap(), Some(newer));
    store.delete_draft("acme-eng").await.unwrap();
    assert_eq!(store.draft("acme-eng").await.unwrap(), None);
}

#[tokio::test]
async fn draft_keeps_the_invite_it_answers() {
    let (_dir, store) = store_with("acme-eng").await;
    let mut kept = draft("acme-eng", 2, "Add sam@acme.example as Member");
    kept.invite_id = Some("inv-1".into());
    store.put_draft(&kept).await.unwrap();
    assert_eq!(store.draft("acme-eng").await.unwrap(), Some(kept));
}

#[tokio::test]
async fn release_invite_makes_a_used_invite_open_again() {
    let (_dir, store) = store_with("acme-eng").await;
    store
        .add_invite(&invite("inv-1", "acme-eng", "2026-10-29T09:00:00.000Z"))
        .await
        .unwrap();
    store
        .mark_invite_used("acme-eng", "inv-1", "2026-10-29T12:00:00.000Z")
        .await
        .unwrap();
    store.release_invite("acme-eng", "inv-1").await.unwrap();
    let got = store.invite("acme-eng", "inv-1").await.unwrap().unwrap();
    assert_eq!(got.used_at, None);

    let err = store
        .release_invite("acme-eng", "missing")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn pending_request_round_trips_and_clears() {
    let (_dir, store) = store_with("acme-eng").await;
    let pending = PendingRequest {
        code: "dots-request-abc".into(),
        invite_id: Some("inv-1".into()),
    };
    store
        .set_pending_request("acme-eng", Some(&pending))
        .await
        .unwrap();
    let row = store.get_org("acme-eng").await.unwrap().unwrap();
    assert_eq!(row.pending_request, Some(pending));
    store.set_pending_request("acme-eng", None).await.unwrap();
    let row = store.get_org("acme-eng").await.unwrap().unwrap();
    assert_eq!(row.pending_request, None);

    let err = store.set_pending_request("ghost", None).await.unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn damaged_pending_request_names_the_org() {
    let (dir, store) = store_with("acme-eng").await;
    let pool = sqlx::sqlite::SqlitePool::connect(&format!(
        "sqlite://{}",
        dir.path().join("dots.db").display()
    ))
    .await
    .unwrap();
    sqlx::query("UPDATE orgs SET pending_request = 'not json' WHERE id = 'acme-eng'")
        .execute(&pool)
        .await
        .unwrap();
    let err = store.get_org("acme-eng").await.unwrap_err();
    assert!(
        matches!(&err, Error::Other(m) if m.contains("acme-eng") && m.contains("pending request")),
        "{err:?}"
    );
}

#[tokio::test]
async fn removing_the_org_removes_its_invites_and_draft() {
    let (_dir, store) = store_with("acme-eng").await;
    store
        .add_invite(&invite("inv-1", "acme-eng", "2026-10-29T09:00:00.000Z"))
        .await
        .unwrap();
    store.put_draft(&draft("acme-eng", 5, "x")).await.unwrap();
    store.remove_org("acme-eng").await.unwrap();
    assert_eq!(store.invite("acme-eng", "inv-1").await.unwrap(), None);
    assert!(store.list_invites("acme-eng").await.unwrap().is_empty());
    assert_eq!(store.draft("acme-eng").await.unwrap(), None);
}
