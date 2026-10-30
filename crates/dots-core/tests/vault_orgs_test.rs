//! The `Orgs` service (env vault spec §6.1, §6.2 and §6.4): joining an org, changing its
//! settings, leaving it, and what each outcome of a fetch does to its status, its values and
//! its offline copy. The fetches go to the fake OpenBao and the SOPS fixtures, and the policies
//! are signed with minisign. No error or summary ever holds a value.

mod common;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::ThreadId;
use std::time::Duration as StdDuration;

use chrono::Duration;
use common::fake_openbao::{Failure, FakeOpenBao};
use common::orgs::{
    at, code, keys, openbao, policy, public_key, sign, sops_at, stamp, token, OrgsHarness,
    Scripted, ACME, ACME_NAME, ISSUED, START, TOKEN,
};
use common::spec;
use dots_core::accounts::Membership;
use dots_core::model::DotSpec;
use dots_core::vault::entries::EntryKind;
use dots_core::vault::org::sources::sops::public_key as age_public_key;
use dots_core::vault::org::{
    key_fingerprint, parse_policy, read_offline, verify_policy, write_offline, BaoAuth,
    InfisicalSettings, OfflineCopy, OpenBaoSettings, OrgEntry, OrgLookup, OrgRow, OrgStatus,
    OrgSummary, OrgView, Orgs, PersonalMode, Protector, SourceCreds, SourceError, SourceSettings,
    TestProtector, UseRules, RETRY_AFTER,
};
use dots_core::vault::{redact, Grants};
use dots_core::{Error, Result};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePool;
use zeroize::Zeroizing;

const API_KEY: &str = "org-secret-api-key-3f9a1c";
const LOG_LEVEL: &str = "org-plain-log-level-77d2";
const ROTATED: &str = "org-secret-rotated-key-8c41e5";
const OTHER_TOKEN: &str = "hvs.other-person-token-6d0f";

/// Every value a test serves, which no error, summary or listing may hold.
const VALUES: &[&str] = &[API_KEY, LOG_LEVEL, ROTATED, TOKEN, OTHER_TOKEN];

fn entries() -> Value {
    json!({"API_KEY": API_KEY, "LOG_LEVEL": LOG_LEVEL})
}

/// `LOG_LEVEL` is plain; Personal entries fill in where the org has no entry.
fn acme_policy() -> Value {
    json!({"plain": ["LOG_LEVEL"], "personal": "fallback"})
}

fn value(value: &str, is_plain: bool) -> OrgLookup {
    OrgLookup::Value {
        value: Zeroizing::new(value.into()),
        is_plain,
    }
}

fn assert_clean(text: &str) {
    for value in VALUES {
        assert!(!text.contains(value), "{text:?} holds {value:?}");
    }
}

/// The error's text, checked for values both ways it can be printed.
fn error_text(e: &Error) -> String {
    assert_clean(&format!("{e:?}"));
    let text = e.to_string();
    assert_clean(&text);
    text
}

fn other(e: Error) -> String {
    error_text(&e);
    match e {
        Error::Other(why) => why,
        e => panic!("expected Error::Other, got {e:?}"),
    }
}

fn invalid(e: Error) -> String {
    let text = error_text(&e);
    match e {
        Error::Invalid(why) => why,
        e => panic!("expected Error::Invalid ({text}), got {e:?}"),
    }
}

fn status(summary: &OrgSummary) -> OrgStatus {
    assert_clean(&serde_json::to_string(summary).unwrap());
    summary.status.clone()
}

async fn status_of(orgs: &Orgs, org_id: &str) -> OrgStatus {
    status(&orgs.summary(org_id).await.expect("the org is listed"))
}

fn copy_of(h: &OrgsHarness, org_id: &str) -> Option<OfflineCopy> {
    read_offline(&h.vault_dir(), &TestProtector, org_id).unwrap()
}

fn names(copy: &OfflineCopy) -> BTreeMap<String, String> {
    copy.entries
        .iter()
        .map(|(k, v)| (k.clone(), v.to_string()))
        .collect()
}

fn stored_creds(h: &OrgsHarness, org_id: &str) -> Option<SourceCreds> {
    SourceCreds::load(h.secrets.as_ref(), org_id).unwrap()
}

/// `START` plus `hours`, as the service writes it.
fn start_plus(hours: i64) -> String {
    stamp(at(START) + Duration::hours(hours))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sops")
        .join(name)
}

/// The secret line of a fixture identity file.
fn fixture_identity(file: &str) -> String {
    std::fs::read_to_string(fixture(file))
        .unwrap()
        .lines()
        .find(|line| line.starts_with("AGE-SECRET-KEY-1"))
        .unwrap()
        .to_string()
}

fn dot_spec(name: &str) -> DotSpec {
    let mut spec = spec(name);
    spec.variables = Grants(BTreeMap::from([
        ("shell".to_string(), vec!["LOG_LEVEL".to_string()]),
        ("mcp:github".to_string(), vec!["API_KEY".to_string()]),
    ]));
    spec
}

// Join

#[tokio::test]
async fn a_join_fetches_verifies_and_keeps_the_row_the_copy_and_the_creds() {
    let h = OrgsHarness::new().await;
    let summary = h.join_acme(entries(), acme_policy()).await;

    let accepted = parse_policy(&policy(ACME, ISSUED, acme_policy())).unwrap();
    let expected = OrgSummary {
        id: ACME.into(),
        name: ACME_NAME.into(),
        source_kind: "openbao".into(),
        address: h.bao.address(),
        key_fingerprint: key_fingerprint(&public_key(&h.keys)).unwrap(),
        status: OrgStatus::Connected,
        status_text: "connected".into(),
        last_fetch_at: Some(start_plus(0)),
        copy_until: Some(start_plus(24)),
        policy_summary: accepted.summary(),
        personal_mode: Some(PersonalMode::Fallback),
        shell_secrets: false,
        // No `use` in the policy: its default, every name.
        use_rules: Some(UseRules::default()),
        membership: Membership::Legacy,
        role: None,
        roster_seq: None,
        roster_error: None,
        member_count: None,
        not_member_text: None,
        request_code: None,
    };
    assert_eq!(summary, expected);
    assert_eq!(h.orgs.summaries().await, vec![expected.clone()]);
    assert_eq!(h.orgs.policy(ACME), Some(accepted));
    let json = serde_json::to_value(&summary).unwrap();
    assert_eq!(json["status"], "connected");
    assert_clean(&json.to_string());

    let row = h.store.get_org(ACME).await.unwrap().unwrap();
    assert_eq!(
        row,
        OrgRow {
            id: ACME.into(),
            name: ACME_NAME.into(),
            source: openbao(&h.bao.address(), "dots"),
            policy_key: public_key(&h.keys),
            last_issued_at: Some(ISSUED.into()),
            last_fetch_at: Some(start_plus(0)),
            joined_at: start_plus(0),
            member: Default::default(),
            pending_request: None,
        }
    );
    assert_eq!(stored_creds(&h, ACME), Some(token()));
    let copy = copy_of(&h, ACME).unwrap();
    assert_eq!(copy.fetched_at, start_plus(0));
    assert_eq!(
        names(&copy),
        BTreeMap::from([
            ("API_KEY".to_string(), API_KEY.to_string()),
            ("LOG_LEVEL".to_string(), LOG_LEVEL.to_string()),
        ])
    );
    verify_policy(&copy.policy, &public_key(&h.keys), ACME).unwrap();

    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(API_KEY, false));
    assert_eq!(
        h.orgs.lookup(ACME, "log_level").await,
        value(LOG_LEVEL, true)
    );
    assert_eq!(h.orgs.lookup(ACME, "MISSING").await, OrgLookup::NotDefined);
    assert_eq!(h.orgs.lookup("beta", "API_KEY").await, OrgLookup::Removed);
}

/// After a failed join, nothing of the org is kept.
async fn assert_nothing_kept(h: &OrgsHarness) {
    assert_eq!(h.store.get_org(ACME).await.unwrap(), None);
    assert_eq!(stored_creds(h, ACME), None);
    assert!(!h.copy_path(ACME).exists());
    assert!(h.orgs.summaries().await.is_empty());
    assert_eq!(h.orgs.summary(ACME).await, None);
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, OrgLookup::Removed);
}

#[tokio::test]
async fn a_join_whose_fetch_or_policy_fails_keeps_nothing() {
    let h = OrgsHarness::new().await;
    h.serve_acme(entries(), acme_policy());

    // The source refuses the token.
    let wrong = SourceCreds::Token {
        token: Zeroizing::new(OTHER_TOKEN.into()),
    };
    let e = h.orgs.join(&h.acme_code(), Some(wrong)).await.unwrap_err();
    assert_eq!(
        other(e),
        format!(
            "could not join Acme Engineering: {} refused access to secret/dots/env (HTTP 403)",
            h.bao.address()
        )
    );
    assert_nothing_kept(&h).await;

    // The source can't be reached.
    let gone = FakeOpenBao::start().await;
    let address = gone.address();
    drop(gone);
    let dead = code(ACME, ACME_NAME, openbao(&address, "dots"), &h.keys);
    let why = other(h.orgs.join(&dead, Some(token())).await.unwrap_err());
    assert!(
        why.starts_with(&format!(
            "could not join Acme Engineering: cannot reach {address}"
        )),
        "{why}"
    );
    assert_nothing_kept(&h).await;

    // The source has nothing at the path.
    let empty = h.code_at(ACME, ACME_NAME, "nothing");
    let why = other(h.orgs.join(&empty, Some(token())).await.unwrap_err());
    assert_eq!(
        why,
        "could not join Acme Engineering: no secrets at secret/nothing/env"
    );
    assert_nothing_kept(&h).await;

    // The policy is signed by another key.
    let signed = sign(&keys(), &policy(ACME, ISSUED, json!({})));
    h.serve("dots", entries(), &signed);
    let why = other(
        h.orgs
            .join(&h.acme_code(), Some(token()))
            .await
            .unwrap_err(),
    );
    assert_eq!(
        why,
        "could not join Acme Engineering: policy invalid: signature does not match"
    );
    assert_nothing_kept(&h).await;

    // The policy is for another org.
    let signed = sign(&h.keys, &policy("other-org", ISSUED, json!({})));
    h.serve("dots", entries(), &signed);
    let why = other(
        h.orgs
            .join(&h.acme_code(), Some(token()))
            .await
            .unwrap_err(),
    );
    assert_eq!(
        why,
        "could not join Acme Engineering: policy invalid: for org 'other-org', not 'acme-eng'"
    );
    assert_nothing_kept(&h).await;

    // Once the source and the policy are right, the join works.
    h.serve_acme(entries(), acme_policy());
    let summary = h.orgs.join(&h.acme_code(), Some(token())).await.unwrap();
    assert_eq!(status(&summary), OrgStatus::Connected);
}

#[tokio::test]
async fn joining_an_org_already_joined_is_refused_and_keeps_its_creds() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    let reads = h.env_reads("dots");

    let other_token = SourceCreds::Token {
        token: Zeroizing::new(OTHER_TOKEN.into()),
    };
    let e = h
        .orgs
        .join(&h.acme_code(), Some(other_token))
        .await
        .unwrap_err();
    assert!(matches!(e, Error::Conflict(_)), "{e:?}");
    assert_eq!(
        error_text(&e),
        "conflict: the org 'acme-eng' is already joined on this PC; use Update to change its \
         settings"
    );
    assert_eq!(h.env_reads("dots"), reads, "nothing is fetched");
    assert_eq!(stored_creds(&h, ACME), Some(token()));
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(API_KEY, false));
}

#[tokio::test]
async fn creds_must_fit_the_source_and_none_is_only_for_sops() {
    let h = OrgsHarness::new().await;
    h.serve_acme(entries(), acme_policy());

    let e = h.orgs.join(&h.acme_code(), None).await.unwrap_err();
    assert_eq!(
        invalid(e),
        "joining Acme Engineering needs your sign-in details for its OpenBao"
    );
    let login = SourceCreds::Login {
        username: Zeroizing::new("ana".into()),
        password: Zeroizing::new(OTHER_TOKEN.into()),
    };
    let e = h.orgs.join(&h.acme_code(), Some(login)).await.unwrap_err();
    assert_eq!(invalid(e), "this org's OpenBao signs in with a token");
    assert!(h.bao.calls().is_empty(), "nothing is fetched");
    assert_nothing_kept(&h).await;
}

#[tokio::test]
async fn rejoining_a_removed_org_re_attaches_its_dots() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    let dot = h
        .store
        .create_dot(&dot_spec("Triage"), Some(ACME), "default")
        .await
        .unwrap();

    h.orgs.remove(ACME).await.unwrap();
    assert_eq!(h.store.get_dot(&dot.id).await.unwrap(), dot);
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, OrgLookup::Removed);

    h.orgs.join(&h.acme_code(), Some(token())).await.unwrap();
    let again = h.store.get_dot(&dot.id).await.unwrap();
    assert_eq!(again, dot);
    let org_id = again.org_id.unwrap();
    assert_eq!(
        h.orgs.lookup(&org_id, "API_KEY").await,
        value(API_KEY, false)
    );
}

// Remove

#[tokio::test]
async fn remove_wipes_the_copy_the_creds_and_the_row_and_the_dots_stay() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    let dot = h
        .store
        .create_dot(&dot_spec("Triage"), Some(ACME), "default")
        .await
        .unwrap();
    assert!(h.copy_path(ACME).exists());

    h.orgs.remove(ACME).await.unwrap();
    assert!(!h.copy_path(ACME).exists());
    assert_eq!(stored_creds(&h, ACME), None);
    assert_eq!(h.store.get_org(ACME).await.unwrap(), None);
    assert!(h.orgs.summaries().await.is_empty());
    assert_eq!(h.orgs.policy(ACME), None);
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, OrgLookup::Removed);
    assert!(matches!(
        h.orgs.entries(ACME).await,
        Err(Error::NotFound(_))
    ));
    let kept = h.store.get_dot(&dot.id).await.unwrap();
    assert_eq!(kept.org_id.as_deref(), Some(ACME));

    let e = h.orgs.remove(ACME).await.unwrap_err();
    assert_eq!(
        error_text(&e),
        "not found: no org 'acme-eng' is joined on this PC"
    );
    let e = h.orgs.refresh(ACME).await.unwrap_err();
    assert!(matches!(e, Error::NotFound(_)), "{e:?}");
}

// Update

#[tokio::test]
async fn the_preview_shows_a_new_org_or_what_an_update_changes() {
    let h = OrgsHarness::new().await;
    let preview = h.orgs.preview_join(&h.acme_code()).unwrap();
    assert_eq!(preview.org_id, ACME);
    assert_eq!(preview.name, ACME_NAME);
    assert_eq!(preview.source_kind, "openbao");
    assert_eq!(preview.address, h.bao.address());
    assert_eq!(
        preview.key_fingerprint,
        key_fingerprint(&public_key(&h.keys)).unwrap()
    );
    assert_eq!(preview.auth, Some(BaoAuth::Token));
    assert!(!preview.existing);
    assert!(preview.changes.is_empty());
    assert!(!preview.key_changed);
    assert!(!preview.source_changed);

    // The sign-in the code names, so the UI asks for the right details; none but OpenBao's.
    let SourceSettings::OpenBao(bao) = openbao(&h.bao.address(), "dots") else {
        unreachable!()
    };
    let ldap = SourceSettings::OpenBao(OpenBaoSettings {
        auth: BaoAuth::Ldap,
        ..bao
    });
    let ldap = code("beta", "Beta", ldap, &h.keys);
    assert_eq!(
        h.orgs.preview_join(&ldap).unwrap().auth,
        Some(BaoAuth::Ldap)
    );
    let infisical = SourceSettings::Infisical(InfisicalSettings {
        site_url: "https://app.infisical.com".into(),
        project_id: "p1".into(),
        environment: "dev".into(),
        secret_path: "/".into(),
    });
    let infisical = code("gamma", "Gamma", infisical, &h.keys);
    let preview = h.orgs.preview_join(&infisical).unwrap();
    assert_eq!(
        (preview.source_kind.as_str(), preview.auth),
        ("infisical", None)
    );
    let sops = code("delta", "Delta", sops_at("C:/sync/delta.enc.json"), &h.keys);
    assert_eq!(h.orgs.preview_join(&sops).unwrap().auth, None);

    h.join_acme(entries(), acme_policy()).await;
    let calls = h.bao.calls().len();
    let unchanged = h.orgs.preview_join(&h.acme_code()).unwrap();
    assert!(unchanged.existing);
    assert!(unchanged.changes.is_empty(), "{:?}", unchanged.changes);
    assert!(!unchanged.key_changed);
    assert!(!unchanged.source_changed);

    let new_keys = keys();
    let moved = code(
        ACME,
        "Acme Eng",
        openbao(&h.bao.address(), "dots2"),
        &new_keys,
    );
    let preview = h.orgs.preview_join(&moved).unwrap();
    assert!(preview.existing);
    assert!(preview.key_changed);
    assert!(preview.source_changed, "a new path");
    assert_eq!(
        preview.changes,
        vec![
            "name changes from Acme Engineering to Acme Eng".to_string(),
            "path changes from dots to dots2".to_string(),
            format!(
                "the policy key changes from {} to {}: the key that signs the org's policy is \
                 new, so accept this code only if your admin told you the key changed",
                key_fingerprint(&public_key(&h.keys)).unwrap(),
                key_fingerprint(&public_key(&new_keys)).unwrap()
            ),
        ]
    );

    let file = fixture("good.enc.json").display().to_string();
    let to_sops = code(ACME, ACME_NAME, sops_at(&file), &h.keys);
    let preview = h.orgs.preview_join(&to_sops).unwrap();
    assert_eq!(
        preview.changes,
        vec![format!(
            "source changes from OpenBao at {} to SOPS file at {file}",
            h.bao.address()
        )]
    );
    assert!(!preview.key_changed, "the same key, another source");
    assert!(preview.source_changed, "another kind of source");

    // A new name or a new key alone keeps the source.
    let renamed = code(
        ACME,
        "Acme Eng",
        openbao(&h.bao.address(), "dots"),
        &new_keys,
    );
    let preview = h.orgs.preview_join(&renamed).unwrap();
    assert!(preview.key_changed);
    assert!(!preview.source_changed);
    // Any setting of the source counts, also one that only says how to sign in.
    let SourceSettings::OpenBao(bao) = openbao(&h.bao.address(), "dots") else {
        unreachable!()
    };
    let settings = [
        OpenBaoSettings {
            address: "http://localhost:1".into(),
            ..bao.clone()
        },
        OpenBaoSettings {
            mount: "kv".into(),
            ..bao.clone()
        },
        OpenBaoSettings {
            auth: BaoAuth::Userpass,
            ..bao.clone()
        },
        OpenBaoSettings {
            auth: BaoAuth::Ldap,
            auth_mount: Some("corp".into()),
            ..bao.clone()
        },
        OpenBaoSettings {
            namespace: Some("eng".into()),
            ..bao.clone()
        },
    ];
    for changed in settings {
        let changed = code(ACME, ACME_NAME, SourceSettings::OpenBao(changed), &h.keys);
        let preview = h.orgs.preview_join(&changed).unwrap();
        assert!(preview.source_changed, "{:?}", preview.changes);
    }
    assert_eq!(h.bao.calls().len(), calls, "a preview fetches nothing");
}

#[tokio::test]
async fn a_failed_update_leaves_the_old_settings_and_values_working() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    let row = h.store.get_org(ACME).await.unwrap().unwrap();

    // Nothing at the new path. A new path takes the sign-in details again.
    let empty = h.code_at(ACME, ACME_NAME, "nothing");
    let e = h
        .orgs
        .update(ACME, &empty, Some(token()))
        .await
        .unwrap_err();
    assert_eq!(
        other(e),
        "could not update Acme Engineering: no secrets at secret/nothing/env"
    );

    // A policy at the new path signed by another key.
    let signed = sign(&keys(), &policy(ACME, "2026-10-22T10:00:00Z", json!({})));
    h.serve("dots2", json!({"API_KEY": ROTATED}), &signed);
    let moved = h.code_at(ACME, ACME_NAME, "dots2");
    let e = h
        .orgs
        .update(ACME, &moved, Some(token()))
        .await
        .unwrap_err();
    assert_eq!(
        other(e),
        "could not update Acme Engineering: policy invalid: signature does not match"
    );

    // A policy older than the one accepted.
    let signed = sign(&h.keys, &policy(ACME, "2026-10-21T08:00:00Z", json!({})));
    h.serve("dots2", json!({"API_KEY": ROTATED}), &signed);
    let e = h
        .orgs
        .update(ACME, &moved, Some(token()))
        .await
        .unwrap_err();
    assert_eq!(
        other(e),
        "could not update Acme Engineering: policy invalid: issued before the last accepted \
         policy (2026-10-21T08:00:00Z, last 2026-10-22T08:00:00Z)"
    );

    // New creds that the new source refuses are not kept.
    let refused = SourceCreds::Token {
        token: Zeroizing::new(OTHER_TOKEN.into()),
    };
    let e = h
        .orgs
        .update(ACME, &moved, Some(refused))
        .await
        .unwrap_err();
    assert!(other(e).contains("refused access"));

    // A code for another org.
    let beta = h.code_at("beta", "Beta", "dots");
    let e = h.orgs.update(ACME, &beta, None).await.unwrap_err();
    assert_eq!(
        invalid(e),
        "this join code is for org 'beta', not 'acme-eng'"
    );

    assert_eq!(h.store.get_org(ACME).await.unwrap().unwrap(), row);
    assert_eq!(stored_creds(&h, ACME), Some(token()));
    assert_eq!(status_of(&h.orgs, ACME).await, OrgStatus::Connected);
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(API_KEY, false));
    assert_eq!(names(&copy_of(&h, ACME).unwrap())["API_KEY"], API_KEY);
    let reads = h.env_reads("dots");
    h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(
        h.env_reads("dots"),
        reads + 1,
        "a refresh still reads the old path"
    );
}

#[tokio::test]
async fn an_update_replaces_the_settings_and_leaves_dots_and_grants_alone() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    let dot = h
        .store
        .create_dot(&dot_spec("Triage"), Some(ACME), "default")
        .await
        .unwrap();

    // A new path and a new key, with the token given again.
    let new_keys = keys();
    let issued = "2026-10-22T10:00:00Z";
    let signed = sign(
        &new_keys,
        &policy(ACME, issued, json!({"offline_hours": 48})),
    );
    h.serve("dots2", json!({"API_KEY": ROTATED}), &signed);
    let moved = code(
        ACME,
        "Acme Eng",
        openbao(&h.bao.address(), "dots2"),
        &new_keys,
    );
    h.advance(Duration::hours(1));
    let summary = h.orgs.update(ACME, &moved, Some(token())).await.unwrap();

    assert_eq!(status(&summary), OrgStatus::Connected);
    assert_eq!(summary.name, "Acme Eng");
    assert_eq!(
        summary.key_fingerprint,
        key_fingerprint(&public_key(&new_keys)).unwrap()
    );
    assert_eq!(summary.copy_until, Some(start_plus(49)));
    let row = h.store.get_org(ACME).await.unwrap().unwrap();
    assert_eq!(row.name, "Acme Eng");
    assert_eq!(row.source, openbao(&h.bao.address(), "dots2"));
    assert_eq!(row.policy_key, public_key(&new_keys));
    assert_eq!(row.last_issued_at.as_deref(), Some(issued));
    assert_eq!(row.last_fetch_at, Some(start_plus(1)));
    assert_eq!(row.joined_at, start_plus(0));
    assert_eq!(stored_creds(&h, ACME), Some(token()));
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(ROTATED, false));
    assert_eq!(
        h.orgs.lookup(ACME, "LOG_LEVEL").await,
        OrgLookup::NotDefined
    );
    assert_eq!(names(&copy_of(&h, ACME).unwrap())["API_KEY"], ROTATED);
    assert_eq!(h.store.get_dot(&dot.id).await.unwrap(), dot);

    // New creds are kept once the new settings work with them.
    h.bao.allow_token(OTHER_TOKEN, false);
    let other_token = SourceCreds::Token {
        token: Zeroizing::new(OTHER_TOKEN.into()),
    };
    h.orgs
        .update(ACME, &moved, Some(other_token.clone()))
        .await
        .unwrap();
    assert_eq!(stored_creds(&h, ACME), Some(other_token));
    assert_eq!(h.store.get_dot(&dot.id).await.unwrap(), dot);

    // The refresh reads the new path with the new key.
    let reads = h.env_reads("dots2");
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(status(&summary), OrgStatus::Connected);
    assert_eq!(h.env_reads("dots2"), reads + 1);
}

#[tokio::test]
async fn an_update_that_changes_the_source_never_sends_the_saved_sign_in_details() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    let row = h.store.get_org(ACME).await.unwrap().unwrap();
    let signed = sign(&h.keys, &policy(ACME, "2026-10-22T10:00:00Z", json!({})));
    h.serve("dots2", json!({"API_KEY": ROTATED}), &signed);

    // Another host: it would get the token before any signature is checked.
    let elsewhere = FakeOpenBao::start().await;
    elsewhere.allow_token(TOKEN, false);
    let moved_host = code(
        ACME,
        ACME_NAME,
        openbao(&elsewhere.address(), "dots"),
        &h.keys,
    );
    // The same host, another path.
    let moved_path = h.code_at(ACME, ACME_NAME, "dots2");
    h.bao.clear_requests();
    for moved in [&moved_host, &moved_path] {
        let e = h.orgs.update(ACME, moved, None).await.unwrap_err();
        assert_eq!(
            invalid(e),
            "Acme Engineering: the source changed; enter the sign-in details again"
        );
    }
    assert!(elsewhere.calls().is_empty(), "{:?}", elsewhere.calls());
    assert!(h.bao.calls().is_empty(), "{:?}", h.bao.calls());
    assert_eq!(h.store.get_org(ACME).await.unwrap().unwrap(), row);
    assert_eq!(stored_creds(&h, ACME), Some(token()));
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(API_KEY, false));

    // With the details given, the Update goes on as before.
    let summary = h
        .orgs
        .update(ACME, &moved_path, Some(token()))
        .await
        .unwrap();
    assert_eq!(status(&summary), OrgStatus::Connected);
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(ROTATED, false));
    assert_eq!(
        h.store.get_org(ACME).await.unwrap().unwrap().source,
        openbao(&h.bao.address(), "dots2")
    );
    // A code for the same source still uses the saved details.
    let reads = h.env_reads("dots2");
    let renamed = code(
        ACME,
        "Acme Eng",
        openbao(&h.bao.address(), "dots2"),
        &h.keys,
    );
    h.orgs.update(ACME, &renamed, None).await.unwrap();
    assert_eq!(h.env_reads("dots2"), reads + 1);
}

#[tokio::test]
async fn a_sops_file_that_moves_keeps_this_pcs_key_and_another_kind_asks_again() {
    let h = OrgsHarness::new().await;
    let scripted = Scripted::new();
    let orgs = h.service().with_sources(scripted.factory());
    let first = code(
        ACME,
        ACME_NAME,
        sops_at("https://files.acme.example/env.json"),
        &h.keys,
    );
    orgs.sops_identity(ACME).await.unwrap();
    let stored = stored_creds(&h, ACME).unwrap();
    let signed = sign(&h.keys, &policy(ACME, ISSUED, acme_policy()));
    scripted.serve(&[("API_KEY", API_KEY)], &signed);
    orgs.join(&first, None).await.unwrap();

    let moved = code(
        ACME,
        ACME_NAME,
        sops_at("C:/Sync/acme/env.enc.json"),
        &h.keys,
    );
    let preview = orgs.preview_join(&moved).unwrap();
    assert!(!preview.source_changed, "{:?}", preview.changes);
    let summary = orgs.update(ACME, &moved, None).await.unwrap();
    assert_eq!(status(&summary), OrgStatus::Connected);
    assert_eq!(summary.address, "C:/Sync/acme/env.enc.json");
    assert_eq!(scripted.creds(), vec![stored.clone(), stored.clone()]);
    assert_eq!(stored_creds(&h, ACME), Some(stored.clone()));

    // From SOPS to OpenBao: the age key is no sign-in for it.
    let to_bao = h.acme_code();
    assert!(orgs.preview_join(&to_bao).unwrap().source_changed);
    let e = orgs.update(ACME, &to_bao, None).await.unwrap_err();
    assert_eq!(
        invalid(e),
        "Acme Engineering: the source changed; enter the sign-in details again"
    );
    assert_eq!(scripted.creds().len(), 2, "nothing is fetched");
    assert!(h.bao.calls().is_empty());
    assert_eq!(stored_creds(&h, ACME), Some(stored));
}

/// No SOPS details exist for an org that was OpenBao, so no creds make the move work.
#[tokio::test]
async fn an_openbao_org_moved_to_sops_is_refused_whatever_the_creds() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    let calls = h.bao.calls().len();
    let to_sops = code(
        ACME,
        ACME_NAME,
        sops_at("https://files.acme.example/env.json"),
        &h.keys,
    );
    for creds in [None, Some(token())] {
        let e = h.orgs.update(ACME, &to_sops, creds).await.unwrap_err();
        assert_eq!(
            invalid(e),
            "Acme Engineering: the source moved to a SOPS file; remove the org and join it \
             again"
        );
    }
    assert_eq!(h.bao.calls().len(), calls, "the fake sees no request");
}

/// A signed URL's query is a token: the row keeps it, to fetch with, and nothing shows it.
#[tokio::test]
async fn a_sops_urls_query_is_kept_but_never_shown() {
    const SIGNED: &str = "https://files.acme.example/env.json?sig=sig-token-4b1e&exp=1";
    const MOVED: &str = "https://files.acme.example/env2.json?sig=sig-token-9c7d#top";
    let h = OrgsHarness::new().await;
    let scripted = Scripted::new();
    let orgs = h.service().with_sources(scripted.factory());
    let first = code(ACME, ACME_NAME, sops_at(SIGNED), &h.keys);
    let mut texts = Vec::new();
    texts.push(format!("{:?}", orgs.preview_join(&first).unwrap()));
    orgs.sops_identity(ACME).await.unwrap();
    scripted.answer(Err(SourceError::Denied(
        "this PC's key is not a recipient of the file".into(),
    )));
    let pending = orgs.join(&first, None).await.unwrap();
    assert_eq!(pending.address, "https://files.acme.example/env.json");
    texts.push(serde_json::to_string(&pending).unwrap());

    let signed = sign(&h.keys, &policy(ACME, ISSUED, acme_policy()));
    scripted.serve(&[("API_KEY", API_KEY)], &signed);
    let summary = orgs.join(&first, None).await.unwrap();
    assert_eq!(status(&summary), OrgStatus::Connected);
    assert_eq!(summary.address, "https://files.acme.example/env.json");
    texts.push(serde_json::to_string(&summary).unwrap());
    texts.push(serde_json::to_string(&orgs.summaries().await).unwrap());
    assert_eq!(
        h.store.get_org(ACME).await.unwrap().unwrap().source,
        sops_at(SIGNED),
        "the row keeps the location as given"
    );

    let moved = code(ACME, ACME_NAME, sops_at(MOVED), &h.keys);
    let preview = orgs.preview_join(&moved).unwrap();
    assert_eq!(preview.address, "https://files.acme.example/env2.json");
    assert_eq!(
        preview.changes,
        vec![
            "location changes from https://files.acme.example/env.json to \
             https://files.acme.example/env2.json"
                .to_string()
        ]
    );
    texts.push(format!("{preview:?}"));
    let to_bao = h.acme_code();
    let preview = orgs.preview_join(&to_bao).unwrap();
    assert_eq!(
        preview.changes,
        vec![format!(
            "source changes from SOPS file at https://files.acme.example/env.json to OpenBao at {}",
            h.bao.address()
        )]
    );
    let http = code(
        ACME,
        ACME_NAME,
        sops_at("http://files.acme.example/env.json?sig=sig-token-0a2f"),
        &h.keys,
    );
    let e = orgs.preview_join(&http).unwrap_err();
    assert_eq!(
        invalid(e),
        "join code: the source's location 'http://files.acme.example/env.json' must start with \
         https:// (http:// only for localhost or 127.0.0.1)"
    );
    for text in &texts {
        assert!(!text.contains("sig-token"), "{text}");
        assert!(!text.contains("exp=1"), "{text}");
    }
}

// Refresh

#[tokio::test]
async fn a_denied_refresh_wipes_the_copy_and_the_values() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    h.bao.revoke_token(TOKEN);

    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(status(&summary), OrgStatus::Revoked);
    assert_eq!(
        summary.status_text,
        format!(
            "access revoked ({} refused access to secret/dots/env (HTTP 403))",
            h.bao.address()
        )
    );
    assert_eq!(summary.copy_until, None);
    assert!(!h.copy_path(ACME).exists());
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, OrgLookup::Revoked);
    assert_eq!(h.orgs.lookup(ACME, "LOG_LEVEL").await, OrgLookup::Revoked);
    assert!(h.orgs.entries(ACME).await.unwrap().is_empty());

    // A load after it has no copy to read.
    let restarted = h.service();
    restarted.load().await.unwrap();
    assert_eq!(
        status_of(&restarted, ACME).await,
        OrgStatus::Offline { until: None }
    );
    assert_eq!(restarted.lookup(ACME, "API_KEY").await, OrgLookup::Offline);

    // Access given back: the next refresh connects again.
    h.bao.allow_token(TOKEN, false);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(status(&summary), OrgStatus::Connected);
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(API_KEY, false));
}

#[tokio::test]
async fn an_unreachable_refresh_keeps_the_copy_until_offline_hours() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    h.advance(Duration::hours(1));
    h.bao.fail_next(Failure::Status(503), 1);

    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(
        status(&summary),
        OrgStatus::Offline {
            until: Some(start_plus(24))
        }
    );
    assert_eq!(
        summary.status_text,
        format!(
            "offline ({} is sealed or not ready (HTTP 503)); using the saved copy until {}",
            h.bao.address(),
            start_plus(24)
        )
    );
    assert_eq!(summary.last_fetch_at, Some(start_plus(0)));
    assert_eq!(summary.copy_until, Some(start_plus(24)));
    assert_eq!(copy_of(&h, ACME).unwrap().fetched_at, start_plus(0));
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(API_KEY, false));

    // Back online, with a rotated value: the next fetch brings it.
    h.serve_acme(json!({"API_KEY": ROTATED}), acme_policy());
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(status(&summary), OrgStatus::Connected);
    assert_eq!(summary.copy_until, Some(start_plus(25)));
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(ROTATED, false));
}

#[tokio::test]
async fn an_invalid_answer_or_a_bad_signature_keeps_the_last_good_copy() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    let bytes = std::fs::read(h.copy_path(ACME)).unwrap();

    h.bao.remove("secret", "dots/env");
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(
        status(&summary),
        OrgStatus::PolicyInvalid {
            error: "no secrets at secret/dots/env".into()
        }
    );
    assert_eq!(
        summary.status_text,
        format!(
            "policy invalid: no secrets at secret/dots/env; using the saved copy until {}",
            start_plus(24)
        )
    );
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(API_KEY, false));

    // New values under a policy signed by another key are not taken.
    let signed = sign(
        &keys(),
        &policy(ACME, "2026-10-22T10:00:00Z", acme_policy()),
    );
    h.serve("dots", json!({"API_KEY": ROTATED}), &signed);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(
        status(&summary),
        OrgStatus::PolicyInvalid {
            error: "signature does not match".into()
        }
    );
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(API_KEY, false));
    assert_eq!(std::fs::read(h.copy_path(ACME)).unwrap(), bytes);
    let row = h.store.get_org(ACME).await.unwrap().unwrap();
    assert_eq!(row.last_issued_at.as_deref(), Some(ISSUED));
}

#[tokio::test]
async fn a_rolled_back_policy_is_refused_and_the_copy_kept() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;

    let older = "2026-10-21T08:00:00Z";
    let signed = sign(&h.keys, &policy(ACME, older, acme_policy()));
    h.serve("dots", json!({"API_KEY": ROTATED}), &signed);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(
        status(&summary),
        OrgStatus::PolicyInvalid {
            error: "issued before the last accepted policy (2026-10-21T08:00:00Z, last \
                    2026-10-22T08:00:00Z)"
                .into()
        }
    );
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(API_KEY, false));
    let row = h.store.get_org(ACME).await.unwrap().unwrap();
    assert_eq!(row.last_issued_at.as_deref(), Some(ISSUED));

    // A newer policy is taken, and recorded.
    let newer = "2026-10-22T10:00:00Z";
    let signed = sign(&h.keys, &policy(ACME, newer, acme_policy()));
    h.serve("dots", json!({"API_KEY": ROTATED}), &signed);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(status(&summary), OrgStatus::Connected);
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(ROTATED, false));
    let row = h.store.get_org(ACME).await.unwrap().unwrap();
    assert_eq!(row.last_issued_at.as_deref(), Some(newer));
}

#[tokio::test]
async fn a_policy_for_another_org_is_refused_and_the_copy_kept() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;

    let signed = sign(
        &h.keys,
        &policy("other-org", "2026-10-22T10:00:00Z", json!({})),
    );
    h.serve("dots", json!({"API_KEY": ROTATED}), &signed);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(
        status(&summary),
        OrgStatus::PolicyInvalid {
            error: "for org 'other-org', not 'acme-eng'".into()
        }
    );
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(API_KEY, false));
    assert_eq!(names(&copy_of(&h, ACME).unwrap())["API_KEY"], API_KEY);
}

#[tokio::test]
async fn refresh_all_fetches_every_org() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    let signed = sign(&h.keys, &policy("beta", ISSUED, json!({})));
    h.serve("beta", json!({"BETA_KEY": ROTATED}), &signed);
    let beta = h.code_at("beta", "Beta", "beta");
    h.orgs.join(&beta, Some(token())).await.unwrap();

    h.orgs.refresh_all().await;
    assert_eq!(h.env_reads("dots"), 2);
    assert_eq!(h.env_reads("beta"), 2);
}

// Expiry

#[tokio::test]
async fn the_copy_expires_offline_hours_after_the_last_good_fetch() {
    let h = OrgsHarness::new().await;
    let extra = json!({"plain": ["LOG_LEVEL"], "offline_hours": 2});
    h.join_acme(entries(), extra.clone()).await;
    h.advance(Duration::hours(1));
    h.bao.fail_next(Failure::Status(503), 1);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(
        status(&summary),
        OrgStatus::Offline {
            until: Some(start_plus(2))
        }
    );

    h.advance(Duration::minutes(59));
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(API_KEY, false));
    h.advance(Duration::minutes(1));
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, OrgLookup::Expired);
    assert!(!h.copy_path(ACME).exists());
    let summary = h.orgs.summary(ACME).await.unwrap();
    assert_eq!(status(&summary), OrgStatus::Expired);
    assert_eq!(
        summary.status_text,
        "secrets expired offline; reconnect to refresh"
    );
    assert_eq!(summary.copy_until, None);
    assert!(h.orgs.entries(ACME).await.unwrap().is_empty());

    // Still offline: still expired.
    h.bao.fail_next(Failure::Status(503), 1);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(status(&summary), OrgStatus::Expired);

    // Reconnected: values again.
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(status(&summary), OrgStatus::Connected);
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(API_KEY, false));
}

#[tokio::test]
async fn an_unreachable_refresh_after_the_copy_ran_out_is_expired() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    h.advance(Duration::hours(24));
    h.bao.fail_next(Failure::Status(503), 1);

    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(status(&summary), OrgStatus::Expired);
    assert!(!h.copy_path(ACME).exists());
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, OrgLookup::Expired);
}

#[tokio::test]
async fn load_uses_a_copy_that_holds_and_drops_one_that_ran_out() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;

    h.advance(Duration::hours(23));
    let restarted = h.service();
    restarted.load().await.unwrap();
    let summary = restarted.summary(ACME).await.unwrap();
    assert_eq!(
        status(&summary),
        OrgStatus::Offline {
            until: Some(start_plus(24))
        }
    );
    assert_eq!(
        summary.status_text,
        format!("offline; using the saved copy until {}", start_plus(24))
    );
    assert_eq!(
        summary.policy_summary,
        h.orgs.policy(ACME).unwrap().summary()
    );
    assert_eq!(
        restarted.lookup(ACME, "API_KEY").await,
        value(API_KEY, false)
    );

    h.advance(Duration::hours(1));
    let restarted = h.service();
    restarted.load().await.unwrap();
    assert_eq!(status_of(&restarted, ACME).await, OrgStatus::Expired);
    assert!(!h.copy_path(ACME).exists());
    assert_eq!(restarted.lookup(ACME, "API_KEY").await, OrgLookup::Expired);
}

#[tokio::test]
async fn offline_hours_0_keeps_no_copy_and_offline_gives_nothing() {
    let h = OrgsHarness::new().await;
    let summary = h
        .join_acme(
            entries(),
            json!({"plain": ["LOG_LEVEL"], "offline_hours": 0}),
        )
        .await;
    assert_eq!(status(&summary), OrgStatus::Connected);
    assert_eq!(summary.copy_until, None);
    assert!(!h.copy_path(ACME).exists());
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, value(API_KEY, false));

    h.bao.fail_next(Failure::Status(503), 1);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(status(&summary), OrgStatus::Offline { until: None });
    assert_eq!(
        summary.status_text,
        format!(
            "offline ({} is sealed or not ready (HTTP 503)), with no saved copy",
            h.bao.address()
        )
    );
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, OrgLookup::Offline);
    assert!(h.orgs.entries(ACME).await.unwrap().is_empty());
    let files: Vec<_> = std::fs::read_dir(h.vault_dir())
        .map(|dir| dir.flatten().map(|e| e.file_name()).collect())
        .unwrap_or_default();
    assert!(files.is_empty(), "{files:?}");
}

#[tokio::test]
async fn a_policy_that_turns_offline_hours_to_0_deletes_the_copy() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    assert!(h.copy_path(ACME).exists());

    let signed = sign(
        &h.keys,
        &policy(ACME, "2026-10-22T10:00:00Z", json!({"offline_hours": 0})),
    );
    h.serve("dots", entries(), &signed);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(status(&summary), OrgStatus::Connected);
    assert!(!h.copy_path(ACME).exists());
}

#[tokio::test]
async fn refresh_if_stale_fetches_only_after_refresh_minutes() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    assert_eq!(h.env_reads("dots"), 1);

    h.advance(Duration::seconds(14 * 60 + 59));
    assert!(!h.orgs.refresh_if_stale(ACME).await.unwrap());
    assert_eq!(h.env_reads("dots"), 1);

    h.advance(Duration::seconds(1));
    assert!(h.orgs.refresh_if_stale(ACME).await.unwrap());
    assert_eq!(h.env_reads("dots"), 2);
    assert!(!h.orgs.refresh_if_stale(ACME).await.unwrap());
    assert_eq!(h.env_reads("dots"), 2);

    // A failed fetch is not a good one, but a run start does not try again within
    // `RETRY_AFTER` of it: a source that hangs would make every start wait for its timeout.
    assert_eq!(RETRY_AFTER, Duration::minutes(5));
    h.advance(Duration::minutes(15));
    h.bao.fail_next(Failure::Status(503), 1);
    assert!(h.orgs.refresh_if_stale(ACME).await.unwrap());
    assert_eq!(h.env_reads("dots"), 3);
    h.advance(Duration::seconds(4 * 60 + 59));
    assert!(!h.orgs.refresh_if_stale(ACME).await.unwrap());
    assert_eq!(h.env_reads("dots"), 3);
    h.advance(Duration::seconds(1));
    assert!(h.orgs.refresh_if_stale(ACME).await.unwrap());
    assert_eq!(h.env_reads("dots"), 4);
    // That one worked: the org is fresh again.
    assert!(!h.orgs.refresh_if_stale(ACME).await.unwrap());
    assert_eq!(h.env_reads("dots"), 4);

    let e = h.orgs.refresh_if_stale("beta").await.unwrap_err();
    assert!(matches!(e, Error::NotFound(_)), "{e:?}");
}

// What a fetch leaves behind

#[tokio::test]
async fn a_fetched_secret_is_redacted_and_a_plain_value_is_not() {
    const SECRET: &str = "org-redact-secret-5be0a7";
    const PLAIN: &str = "org-redact-plain-c41d09";
    let h = OrgsHarness::new().await;
    h.join_acme(
        json!({"REDACT_ME": SECRET, "LOG_LEVEL": PLAIN}),
        acme_policy(),
    )
    .await;

    let text = format!("token={SECRET} level={PLAIN}");
    assert_eq!(
        redact::global().scrub(&text),
        format!("token=[secret:REDACT_ME] level={PLAIN}")
    );
}

#[tokio::test]
async fn load_adds_the_secrets_of_a_copy_to_the_redactor() {
    const SECRET: &str = "org-loaded-secret-0e93b1";
    const PLAIN: &str = "org-loaded-plain-7a62f4";
    let h = OrgsHarness::new().await;
    let signed = sign(&h.keys, &policy(ACME, ISSUED, acme_policy()));
    h.store
        .add_org(&OrgRow {
            id: ACME.into(),
            name: ACME_NAME.into(),
            source: openbao(&h.bao.address(), "dots"),
            policy_key: public_key(&h.keys),
            last_issued_at: Some(ISSUED.into()),
            last_fetch_at: Some(start_plus(0)),
            joined_at: start_plus(0),
            member: Default::default(),
            pending_request: None,
        })
        .await
        .unwrap();
    let copy = OfflineCopy {
        fetched_at: start_plus(0),
        entries: BTreeMap::from([
            ("LOADED".to_string(), Zeroizing::new(SECRET.to_string())),
            ("LOG_LEVEL".to_string(), Zeroizing::new(PLAIN.to_string())),
        ]),
        policy: signed,
        roster: None,
    };
    write_offline(&h.vault_dir(), &TestProtector, ACME, &copy).unwrap();

    h.orgs.load().await.unwrap();
    assert_eq!(h.orgs.lookup(ACME, "LOADED").await, value(SECRET, false));
    assert_eq!(
        redact::global().scrub(&format!("{SECRET} {PLAIN}")),
        format!("[secret:LOADED] {PLAIN}")
    );
}

#[tokio::test]
async fn a_damaged_org_row_does_not_stop_load() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    let signed = sign(&h.keys, &policy("beta", ISSUED, json!({})));
    h.serve("beta", json!({"BETA_KEY": ROTATED}), &signed);
    let beta = h.code_at("beta", "Beta", "beta");
    h.orgs.join(&beta, Some(token())).await.unwrap();

    let pool = SqlitePool::connect(&format!(
        "sqlite://{}",
        h.data.path().join("dots.db").display()
    ))
    .await
    .unwrap();
    sqlx::query("UPDATE orgs SET source = ? WHERE id = 'beta'")
        .bind(json!({"kind": "dropbox", "folder": "x"}).to_string())
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    let restarted = h.service();
    restarted.load().await.unwrap();
    let summaries = restarted.summaries().await;
    assert_eq!(
        summaries.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(),
        [ACME, "beta"]
    );
    let damaged = &summaries[1];
    assert_eq!(damaged.name, "Beta");
    let OrgStatus::PolicyInvalid { error } = status(damaged) else {
        panic!("{damaged:?}");
    };
    assert!(error.starts_with("its settings are damaged: "), "{error}");
    assert_eq!(damaged.status_text, format!("policy invalid: {error}"));
    assert_eq!(
        restarted.lookup("beta", "BETA_KEY").await,
        OrgLookup::PolicyInvalid(error.clone())
    );
    assert_eq!(
        restarted.lookup(ACME, "API_KEY").await,
        value(API_KEY, false)
    );

    restarted.refresh_all().await;
    assert_eq!(status_of(&restarted, ACME).await, OrgStatus::Connected);
    assert_eq!(
        status_of(&restarted, "beta").await,
        OrgStatus::PolicyInvalid { error }
    );
    let e = restarted.update("beta", &beta, None).await.unwrap_err();
    assert_eq!(
        invalid(e),
        "the settings of org 'beta' are damaged; remove it and join it again"
    );

    restarted.remove("beta").await.unwrap();
    assert!(h.store.list_orgs().await.is_ok(), "the damaged row is gone");
    assert_eq!(stored_creds(&h, "beta"), None);
    assert!(!h.copy_path("beta").exists());
}

#[tokio::test]
async fn load_sweeps_leftover_temp_files() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    let left = [
        h.vault_dir()
            .join(".acme-eng.01JBCDEFGHJKMNPQRSTVWXYZ00.tmp"),
        h.vault_dir().join("stray.tmp"),
    ];
    for path in &left {
        std::fs::write(path, b"protected bytes").unwrap();
    }

    h.service().load().await.unwrap();
    for path in &left {
        assert!(!path.exists(), "{} is left", path.display());
    }
    assert!(h.copy_path(ACME).exists());
}

/// `TestProtector`, recording the thread of each call.
#[derive(Debug, Default)]
struct Tracking(Mutex<Vec<ThreadId>>);

impl Protector for Tracking {
    fn protect(&self, data: &[u8], entropy: &[u8]) -> Result<Vec<u8>> {
        self.0.lock().unwrap().push(std::thread::current().id());
        TestProtector.protect(data, entropy)
    }

    fn unprotect(&self, data: &[u8], entropy: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        self.0.lock().unwrap().push(std::thread::current().id());
        TestProtector.unprotect(data, entropy)
    }
}

#[tokio::test]
async fn the_offline_copy_is_read_and_written_off_the_async_thread() {
    let tracking = Arc::new(Tracking::default());
    let h = OrgsHarness::with_protector(tracking.clone()).await;
    h.join_acme(entries(), acme_policy()).await;
    h.service().load().await.unwrap();

    // This test runs on one thread; a blocking task runs on another.
    let here = std::thread::current().id();
    let threads = tracking.0.lock().unwrap().clone();
    assert_eq!(threads.len(), 2, "one write, one read");
    assert!(threads.iter().all(|thread| *thread != here));
}

#[tokio::test]
async fn entries_show_plain_values_only_and_lock_as_personal_says() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    let listing = h.orgs.entries(ACME).await.unwrap();
    assert_eq!(
        listing,
        vec![
            OrgEntry {
                name: "API_KEY".into(),
                kind: EntryKind::Secret,
                value: None,
                locked: true,
                used_by: vec![],
            },
            OrgEntry {
                name: "LOG_LEVEL".into(),
                kind: EntryKind::Plain,
                value: Some(LOG_LEVEL.into()),
                locked: true,
                used_by: vec![],
            },
        ]
    );
    assert!(!serde_json::to_string(&listing).unwrap().contains(API_KEY));

    for (issued, personal, locked) in [
        ("2026-10-22T10:00:00Z", "override", false),
        ("2026-10-22T11:00:00Z", "deny", true),
    ] {
        let extra = json!({"plain": ["LOG_LEVEL"], "personal": personal});
        h.serve(
            "dots",
            entries(),
            &sign(&h.keys, &policy(ACME, issued, extra)),
        );
        h.orgs.refresh(ACME).await.unwrap();
        let listing = h.orgs.entries(ACME).await.unwrap();
        assert!(listing.iter().all(|e| e.locked == locked), "{personal}");
    }
}

// SOPS

#[tokio::test]
async fn sops_identity_is_made_once_and_kept() {
    let h = OrgsHarness::new().await;
    let key = h.orgs.sops_identity(ACME).await.unwrap();
    assert!(key.starts_with("age1"), "{key}");
    assert_eq!(h.orgs.sops_identity(ACME).await.unwrap(), key);
    let Some(SourceCreds::Age { identity }) = stored_creds(&h, ACME) else {
        panic!("no age key is stored");
    };
    assert_eq!(age_public_key(&identity).unwrap(), key);

    let e = h.orgs.sops_identity("Bad/Id").await.unwrap_err();
    assert!(matches!(e, Error::Invalid(_)), "{e:?}");
}

#[tokio::test]
async fn sops_identity_is_refused_for_an_org_that_reads_another_source() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    let e = h.orgs.sops_identity(ACME).await.unwrap_err();
    assert_eq!(
        invalid(e),
        "org 'acme-eng' does not read a SOPS file, so it has no age key"
    );
    assert_eq!(stored_creds(&h, ACME), Some(token()));
}

#[tokio::test]
async fn a_sops_join_before_the_admin_adds_the_key_waits_and_keeps_the_key() {
    let h = OrgsHarness::new().await;
    let file = fixture("good.enc.json").display().to_string();
    let sops_code = code(ACME, ACME_NAME, sops_at(&file), &h.keys);
    let key = h.orgs.sops_identity(ACME).await.unwrap();

    // Not an error: the answer says the join waits.
    let waiting = h.orgs.join(&sops_code, None).await.unwrap();
    assert_eq!(status(&waiting), OrgStatus::Pending);
    assert_eq!(waiting.id, ACME);
    assert_eq!(waiting.name, ACME_NAME);
    assert_eq!(
        waiting.status_text,
        "waiting for the admin to add this PC's key"
    );
    assert!(h.orgs.summaries().await.is_empty());
    assert_eq!(h.store.get_org(ACME).await.unwrap(), None);
    assert!(!h.copy_path(ACME).exists());
    let pending = h.orgs.summary(ACME).await.unwrap();
    assert_eq!(status(&pending), OrgStatus::Pending);
    assert_eq!(
        pending.status_text,
        "waiting for the admin to add this PC's key"
    );
    assert_eq!(pending.source_kind, "sops");
    assert_eq!(pending.address, file);
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, OrgLookup::Removed);
    assert_eq!(
        h.orgs.sops_identity(ACME).await.unwrap(),
        key,
        "the key stays"
    );

    // Remove takes the key back.
    h.orgs.remove(ACME).await.unwrap();
    assert_eq!(stored_creds(&h, ACME), None);
    assert_eq!(h.orgs.summary(ACME).await, None);
}

#[tokio::test]
async fn a_sops_join_whose_file_opens_but_whose_policy_fails_keeps_the_key() {
    let h = OrgsHarness::new().await;
    let identity = fixture_identity("test-identity.txt");
    let creds = SourceCreds::Age {
        identity: Zeroizing::new(identity.clone()),
    };
    creds.save(h.secrets.as_ref(), ACME).unwrap();
    assert_eq!(
        h.orgs.sops_identity(ACME).await.unwrap(),
        age_public_key(&identity).unwrap()
    );

    // The fixture's policy is a placeholder, not a signed text.
    let file = fixture("good.enc.json").display().to_string();
    let sops_code = code(ACME, ACME_NAME, sops_at(&file), &h.keys);
    let why = other(h.orgs.join(&sops_code, None).await.unwrap_err());
    assert!(
        why.starts_with(
            "could not join Acme Engineering: policy invalid: the signed policy is \
                         not readable: "
        ),
        "{why}"
    );
    assert!(!why.contains("placeholder-signed-policy"), "{why}");
    assert_eq!(h.store.get_org(ACME).await.unwrap(), None);
    assert_eq!(h.orgs.summary(ACME).await, None);
    assert_eq!(stored_creds(&h, ACME), Some(creds));
}

#[tokio::test]
async fn a_sops_join_once_the_key_is_a_recipient_uses_the_stored_key() {
    let h = OrgsHarness::new().await;
    let scripted = Scripted::new();
    let orgs = h.service().with_sources(scripted.factory());
    let sops_code = code(
        ACME,
        ACME_NAME,
        sops_at("https://files.acme.example/env.json"),
        &h.keys,
    );

    let e = orgs.join(&sops_code, None).await.unwrap_err();
    assert_eq!(
        invalid(e),
        "this PC has no age key for org 'acme-eng' yet; make one, send it to your admin, then join"
    );
    assert!(scripted.creds().is_empty());

    orgs.sops_identity(ACME).await.unwrap();
    let stored = stored_creds(&h, ACME).unwrap();
    let signed = sign(&h.keys, &policy(ACME, ISSUED, acme_policy()));
    scripted.serve(&[("API_KEY", API_KEY)], &signed);
    let summary = orgs.join(&sops_code, None).await.unwrap();
    assert_eq!(status(&summary), OrgStatus::Connected);
    assert_eq!(summary.source_kind, "sops");
    assert_eq!(scripted.creds(), vec![stored]);
    assert_eq!(orgs.lookup(ACME, "API_KEY").await, value(API_KEY, false));
}

#[tokio::test]
async fn a_scripted_source_error_maps_to_its_status() {
    let h = OrgsHarness::new().await;
    let scripted = Scripted::new();
    let orgs = h.service().with_sources(scripted.factory());
    let signed = sign(&h.keys, &policy(ACME, ISSUED, acme_policy()));
    scripted.serve(&[("API_KEY", API_KEY)], &signed);
    orgs.join(&h.acme_code(), Some(token())).await.unwrap();

    let cases = [
        (
            SourceError::Invalid("a scripted answer Dots can't use".into()),
            OrgStatus::PolicyInvalid {
                error: "a scripted answer Dots can't use".into(),
            },
            value(API_KEY, false),
        ),
        (
            SourceError::Unreachable("scripted outage".into()),
            OrgStatus::Offline {
                until: Some(start_plus(24)),
            },
            value(API_KEY, false),
        ),
        (
            SourceError::Denied("scripted refusal".into()),
            OrgStatus::Revoked,
            OrgLookup::Revoked,
        ),
    ];
    for (error, expected, lookup) in cases {
        scripted.answer(Err(error));
        let summary = orgs.refresh(ACME).await.unwrap();
        assert_eq!(status(&summary), expected);
        assert_eq!(orgs.lookup(ACME, "API_KEY").await, lookup);
    }
}

// Values stay out of what the service says

#[tokio::test]
async fn errors_summaries_and_listings_never_hold_a_value() {
    let h = OrgsHarness::new().await;
    let mut texts = Vec::new();
    h.serve_acme(entries(), acme_policy());
    let wrong = SourceCreds::Token {
        token: Zeroizing::new(OTHER_TOKEN.into()),
    };
    let e = h.orgs.join(&h.acme_code(), Some(wrong)).await.unwrap_err();
    texts.push(format!("{e} {e:?}"));

    let summary = h.join_acme(entries(), acme_policy()).await;
    texts.push(serde_json::to_string(&summary).unwrap());
    texts.push(format!("{summary:?}"));
    texts.push(format!("{:?}", h.orgs));
    texts.push(format!("{:?}", h.orgs.lookup(ACME, "API_KEY").await));
    texts.push(format!(
        "{:?}",
        h.orgs.preview_join(&h.acme_code()).unwrap()
    ));
    let listing = h.orgs.entries(ACME).await.unwrap();
    texts.push(format!(
        "{:?}",
        listing.iter().find(|e| e.name == "API_KEY")
    ));

    let empty = h.code_at(ACME, ACME_NAME, "nothing");
    let e = h
        .orgs
        .update(ACME, &empty, Some(token()))
        .await
        .unwrap_err();
    texts.push(format!("{e} {e:?}"));

    h.bao.fail_next(Failure::Status(503), 1);
    texts.push(serde_json::to_string(&h.orgs.refresh(ACME).await.unwrap()).unwrap());
    h.bao.remove("secret", "dots/env");
    texts.push(serde_json::to_string(&h.orgs.refresh(ACME).await.unwrap()).unwrap());
    h.bao.revoke_token(TOKEN);
    texts.push(serde_json::to_string(&h.orgs.refresh(ACME).await.unwrap()).unwrap());
    texts.push(serde_json::to_string(&h.orgs.summaries().await).unwrap());

    for text in &texts {
        assert_clean(text);
    }
    assert!(texts[3].contains(ACME), "{}", texts[3]);
}

// Concurrency

#[tokio::test]
async fn a_lookup_runs_while_a_refresh_waits_on_the_network() {
    let h = OrgsHarness::new().await;
    let scripted = Scripted::new();
    let orgs = h.service().with_sources(scripted.factory());
    let signed = sign(&h.keys, &policy(ACME, ISSUED, acme_policy()));
    scripted.serve(&[("API_KEY", API_KEY)], &signed);
    orgs.join(&h.acme_code(), Some(token())).await.unwrap();
    scripted.entered().await;

    let gate = scripted.hold();
    scripted.serve(&[("API_KEY", ROTATED)], &signed);
    let refreshing = tokio::spawn({
        let orgs = orgs.clone();
        async move { orgs.refresh(ACME).await }
    });
    scripted.entered().await;

    let soon = StdDuration::from_secs(2);
    let found = tokio::time::timeout(soon, orgs.lookup(ACME, "API_KEY"))
        .await
        .expect("a lookup does not wait for the refresh");
    assert_eq!(found, value(API_KEY, false));
    tokio::time::timeout(soon, orgs.summaries()).await.unwrap();
    tokio::time::timeout(soon, orgs.entries(ACME))
        .await
        .unwrap()
        .unwrap();
    assert!(orgs.policy(ACME).is_some());

    gate.add_permits(1);
    let summary = refreshing.await.unwrap().unwrap();
    assert_eq!(status(&summary), OrgStatus::Connected);
    assert_eq!(orgs.lookup(ACME, "API_KEY").await, value(ROTATED, false));
}

// One view: the policy and the values of the same fetch

fn asked(list: &[&str]) -> Vec<String> {
    list.iter().map(|n| n.to_string()).collect()
}

/// `view` answers from one read of the org: while a refresh that brings a new policy and new
/// values waits on the network, the view is the old fetch's policy with the old values; once it
/// lands, the new policy with the new values. Never one fetch's values with another's policy.
#[tokio::test]
async fn a_view_gives_the_policy_and_the_values_of_one_fetch() {
    let h = OrgsHarness::new().await;
    let scripted = Scripted::new();
    let orgs = h.service().with_sources(scripted.factory());
    let first = sign(&h.keys, &policy(ACME, ISSUED, acme_policy()));
    scripted.serve(&[("API_KEY", API_KEY), ("LOG_LEVEL", LOG_LEVEL)], &first);
    orgs.join(&h.acme_code(), Some(token())).await.unwrap();
    scripted.entered().await;
    let asked = asked(&["api_key", "LOG_LEVEL", "NOT_THERE"]);

    let view = orgs.view(ACME, &asked).await;
    let policy_of = |view: &OrgView| view.policy.clone().expect("a policy");
    assert_eq!(policy_of(&view).issued_at, ISSUED);
    assert_eq!(policy_of(&view).personal, PersonalMode::Fallback);
    assert_eq!(
        view.lookups,
        [
            value(API_KEY, false),
            value(LOG_LEVEL, true),
            OrgLookup::NotDefined
        ]
    );

    // The next fetch makes API_KEY plain, LOG_LEVEL gone, and Personal denied.
    const LATER: &str = "2026-10-22T08:30:00Z";
    let gate = scripted.hold();
    let second = json!({"plain": ["API_KEY"], "personal": "deny"});
    scripted.serve(
        &[("API_KEY", ROTATED)],
        &sign(&h.keys, &policy(ACME, LATER, second)),
    );
    let refreshing = tokio::spawn({
        let orgs = orgs.clone();
        async move { orgs.refresh(ACME).await }
    });
    scripted.entered().await;
    assert_eq!(
        orgs.view(ACME, &asked).await,
        view,
        "the fetch has not landed"
    );

    gate.add_permits(1);
    refreshing.await.unwrap().unwrap();
    let view = orgs.view(ACME, &asked).await;
    assert_eq!(policy_of(&view).issued_at, LATER);
    assert_eq!(policy_of(&view).personal, PersonalMode::Deny);
    assert_eq!(
        view.lookups,
        [
            value(ROTATED, true),
            OrgLookup::NotDefined,
            OrgLookup::NotDefined
        ]
    );
    assert!(!format!("{view:?}").contains(ROTATED));
}

/// `view` expires a copy that ran out, as `lookup` does, and answers `Removed` for an org that
/// is not joined.
#[tokio::test]
async fn a_view_expires_as_lookup_does_and_names_why_there_are_no_values() {
    let h = OrgsHarness::new().await;
    let extra = json!({"plain": ["LOG_LEVEL"], "offline_hours": 2});
    h.join_acme(entries(), extra).await;
    let asked = asked(&["API_KEY", "LOG_LEVEL"]);
    h.advance(Duration::hours(1));
    h.bao.fail_next(Failure::Status(503), 1);
    h.orgs.refresh(ACME).await.unwrap();

    h.advance(Duration::minutes(59));
    let view = h.orgs.view(ACME, &asked).await;
    assert_eq!(
        view.lookups,
        [value(API_KEY, false), value(LOG_LEVEL, true)]
    );
    h.advance(Duration::minutes(1));
    let view = h.orgs.view(ACME, &asked).await;
    assert_eq!(view.lookups, [OrgLookup::Expired, OrgLookup::Expired]);
    assert!(!h.copy_path(ACME).exists(), "the view deleted the copy");
    assert_eq!(
        status_of(&h.orgs, ACME).await,
        OrgStatus::Expired,
        "as lookup would have left it"
    );

    let view = h.orgs.view("beta", &asked).await;
    assert_eq!(
        view,
        OrgView {
            policy: None,
            lookups: vec![OrgLookup::Removed, OrgLookup::Removed],
        }
    );
    assert!(h.orgs.view(ACME, &[]).await.lookups.is_empty());
}

// Fix round 1

/// Rewrites the saved copy of Acme as if it were fetched at `fetched_at`.
fn date_copy(h: &OrgsHarness, fetched_at: String) {
    let mut copy = copy_of(h, ACME).unwrap();
    copy.fetched_at = fetched_at;
    write_offline(&h.vault_dir(), &TestProtector, ACME, &copy).unwrap();
}

#[tokio::test]
async fn load_drops_a_copy_dated_more_than_5_minutes_ahead() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;

    // A little ahead of the clock is skew, and the copy still serves.
    date_copy(&h, stamp(at(START) + Duration::minutes(2)));
    let restarted = h.service();
    restarted.load().await.unwrap();
    assert!(matches!(
        status_of(&restarted, ACME).await,
        OrgStatus::Offline { .. }
    ));
    assert_eq!(
        restarted.lookup(ACME, "API_KEY").await,
        value(API_KEY, false)
    );

    // A month ahead can't be counted from: the copy is expired, and gone.
    date_copy(&h, stamp(at(START) + Duration::days(30)));
    let restarted = h.service();
    restarted.load().await.unwrap();
    assert_eq!(status_of(&restarted, ACME).await, OrgStatus::Expired);
    assert!(!h.copy_path(ACME).exists());
    assert_eq!(restarted.lookup(ACME, "API_KEY").await, OrgLookup::Expired);
    let summary = restarted.summary(ACME).await.unwrap();
    assert_eq!(summary.copy_until, None);
}

#[tokio::test]
async fn load_deletes_a_copy_it_refuses_or_cannot_read() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;

    // Signed by another key.
    let mut copy = copy_of(&h, ACME).unwrap();
    copy.policy = sign(&keys(), &policy(ACME, ISSUED, acme_policy()));
    write_offline(&h.vault_dir(), &TestProtector, ACME, &copy).unwrap();
    let restarted = h.service();
    restarted.load().await.unwrap();
    assert!(matches!(
        status_of(&restarted, ACME).await,
        OrgStatus::PolicyInvalid { .. }
    ));
    assert!(!h.copy_path(ACME).exists());
    assert!(matches!(
        restarted.lookup(ACME, "API_KEY").await,
        OrgLookup::PolicyInvalid(_)
    ));

    // Bytes that are no copy.
    h.orgs.refresh(ACME).await.unwrap();
    assert!(h.copy_path(ACME).exists());
    std::fs::write(h.copy_path(ACME), b"not a copy").unwrap();
    let restarted = h.service();
    restarted.load().await.unwrap();
    assert!(!h.copy_path(ACME).exists());
    assert_eq!(
        status_of(&restarted, ACME).await,
        OrgStatus::Offline { until: None }
    );
}

#[tokio::test]
async fn an_unreachable_refresh_keeps_a_revoked_org_revoked() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), acme_policy()).await;
    h.bao.revoke_token(TOKEN);
    let revoked = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(status(&revoked), OrgStatus::Revoked);

    h.bao.fail_next(Failure::Status(503), 1);
    let summary = h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(status(&summary), OrgStatus::Revoked);
    assert_eq!(summary.status_text, revoked.status_text);
    assert_eq!(h.orgs.lookup(ACME, "API_KEY").await, OrgLookup::Revoked);
}

/// A lookup that finds the copy due while a refresh holds the org's lock must not lose the
/// delete: the refresh's failed fetch deletes the copy.
#[tokio::test]
async fn an_expiry_met_during_a_refresh_still_deletes_the_copy() {
    let h = OrgsHarness::new().await;
    let scripted = Scripted::new();
    let orgs = h.service().with_sources(scripted.factory());
    let signed = sign(&h.keys, &policy(ACME, ISSUED, acme_policy()));
    scripted.serve(&[("API_KEY", API_KEY)], &signed);
    orgs.join(&h.acme_code(), Some(token())).await.unwrap();
    scripted.entered().await;
    assert!(h.copy_path(ACME).exists());

    // Offline first (a connected org keeps its values), then a refresh that waits.
    scripted.answer(Err(SourceError::Unreachable("down".into())));
    let offline = orgs.refresh(ACME).await.unwrap();
    assert!(matches!(status(&offline), OrgStatus::Offline { .. }));
    scripted.entered().await;
    let gate = scripted.hold();
    let refreshing = tokio::spawn({
        let orgs = orgs.clone();
        async move { orgs.refresh(ACME).await }
    });
    scripted.entered().await;

    h.advance(Duration::hours(24));
    assert_eq!(orgs.lookup(ACME, "API_KEY").await, OrgLookup::Expired);
    assert_eq!(orgs.summary(ACME).await.unwrap().copy_until, None);
    assert!(h.copy_path(ACME).exists(), "the lock holder deletes it");

    gate.add_permits(1);
    let summary = refreshing.await.unwrap().unwrap();
    assert_eq!(status(&summary), OrgStatus::Expired);
    assert!(!h.copy_path(ACME).exists());
    assert_eq!(orgs.lookup(ACME, "API_KEY").await, OrgLookup::Expired);
}
