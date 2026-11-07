//! A record for each fetch outcome of an org (env vault spec §10): `fetched`, `offline`,
//! `revoked`, `expired` and `policy_invalid`, queued while the policy has `audit` and only
//! listed in History without it. History leaves `fetched` out. Leaving the org keeps its queue
//! and chain, and joining again goes on from the next `seq`.

mod common;

use chrono::Duration;
use common::fake_openbao::Failure;
use common::orgs::{keys, policy, sign, OrgsHarness, ACME, ISSUED};
use dots_core::store::HistoryState;
use dots_core::vault::audit::record::{REASON_DENIED, REASON_OFFLINE_COPY};
use dots_core::vault::audit::writer::{machine_name, windows_user};
use dots_core::vault::audit::{
    line_hash, verify_lines, AuditRecord, AuditUser, Event, Happened, Usage,
};
use dots_core::vault::org::sources::sops;
use dots_core::vault::org::SourceCreds;
use dots_core::vault::{Consumer, DotScope, Grants, Unresolved, Vault};
use serde_json::{json, Value};
use zeroize::Zeroizing;

const AUDIT_TOKEN: &str = "audit-token-value-7d41c2";
const API_KEY: &str = "org-secret-api-key-0b9e55";

fn entries() -> Value {
    json!({"API_KEY": API_KEY, "DOTS_AUDIT_TOKEN": AUDIT_TOKEN})
}

fn audited() -> Value {
    json!({"audit": {"endpoint": "https://audit.acme.test/dots", "token_entry": "DOTS_AUDIT_TOKEN"}})
}

async fn queued(h: &OrgsHarness) -> Vec<AuditRecord> {
    let batch = h.store.audit_batch(ACME, 500).await.unwrap();
    for (_, line) in &batch {
        assert!(
            !line.contains(AUDIT_TOKEN) && !line.contains(API_KEY),
            "{line}"
        );
    }
    batch
        .iter()
        .map(|(_, line)| AuditRecord::parse(line).unwrap())
        .collect()
}

async fn history(h: &OrgsHarness) -> Vec<(Event, Option<String>, HistoryState)> {
    h.store
        .audit_history(ACME, 100)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.event, row.reason, row.state))
        .collect()
}

#[tokio::test]
async fn a_join_queues_a_fetched_record_that_history_leaves_out() {
    let h = OrgsHarness::new().await;
    let identity = dots_core::accounts::ensure_identity(&h.store, h.secrets.as_ref())
        .await
        .unwrap();
    h.join_acme(entries(), audited()).await;

    let records = queued(&h).await;
    assert_eq!(records.len(), 1);
    let r = &records[0];
    assert_eq!((r.seq, r.prev.clone(), r.event), (1, None, Event::Fetched));
    assert_eq!(r.at, "2026-10-22T09:00:00Z");
    assert_eq!(r.org_id, ACME);
    assert_eq!(r.install, h.store.install_id().await.unwrap());
    assert_eq!(
        r.user,
        AuditUser {
            windows: windows_user(),
            machine: machine_name(),
            source_identity: None,
            dots_id: Some(identity.fingerprint),
        }
    );
    assert_eq!(r.source.as_deref(), Some("openbao"));
    assert_eq!((r.dot.clone(), r.run_id.clone()), (None, None));
    assert_eq!(
        (r.name.clone(), r.consumer.clone(), r.reason.clone()),
        (None, None, None)
    );
    assert_eq!(history(&h).await, vec![]);
}

#[tokio::test]
async fn each_failed_fetch_is_recorded_with_its_outcome() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), audited()).await;

    h.bao.fail_next(Failure::Status(503), 1);
    h.orgs.refresh(ACME).await.unwrap();
    // A policy signed with another key.
    let forged = sign(&keys(), &policy(ACME, ISSUED, audited()));
    h.serve("dots", entries(), &forged);
    h.orgs.refresh(ACME).await.unwrap();
    h.bao.revoke_token(common::orgs::TOKEN);
    h.orgs.refresh(ACME).await.unwrap();

    let events: Vec<(u64, Event, Option<String>)> = queued(&h)
        .await
        .into_iter()
        .map(|r| (r.seq, r.event, r.reason))
        .collect();
    assert_eq!(
        events,
        vec![
            (1, Event::Fetched, None),
            (2, Event::Offline, None),
            (3, Event::PolicyInvalid, None),
            (4, Event::Revoked, Some(REASON_DENIED.into())),
        ]
    );
    assert_eq!(
        history(&h).await,
        vec![
            (
                Event::Revoked,
                Some(REASON_DENIED.into()),
                HistoryState::Queued
            ),
            (Event::PolicyInvalid, None, HistoryState::Queued),
            (Event::Offline, None, HistoryState::Queued),
        ]
    );
}

#[tokio::test]
async fn a_copy_that_runs_out_is_recorded_once_and_at_start() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), audited()).await;
    h.bao.fail_next(Failure::Status(503), 1);
    h.orgs.refresh(ACME).await.unwrap();
    h.advance(Duration::hours(25));
    h.orgs.lookup(ACME, "API_KEY").await;
    h.orgs.lookup(ACME, "API_KEY").await;
    let events: Vec<(Event, Option<String>)> = queued(&h)
        .await
        .into_iter()
        .map(|r| (r.event, r.reason))
        .collect();
    assert_eq!(
        events,
        vec![
            (Event::Fetched, None),
            (Event::Offline, None),
            (Event::Expired, Some(REASON_OFFLINE_COPY.into())),
        ]
    );

    // A copy that ran out while Dots was closed is recorded when it starts.
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), audited()).await;
    h.advance(Duration::hours(25));
    let again = h.service();
    again.load().await.unwrap();
    let last = queued(&h).await.pop().unwrap();
    assert_eq!(
        (last.seq, last.event, last.reason),
        (2, Event::Expired, Some(REASON_OFFLINE_COPY.into()))
    );
}

#[tokio::test]
async fn without_audit_in_the_policy_nothing_is_queued_and_history_still_lists() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), json!({})).await;
    h.bao.fail_next(Failure::Status(503), 1);
    h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(queued(&h).await, vec![]);
    assert_eq!(
        history(&h).await,
        vec![(Event::Offline, None, HistoryState::Local)]
    );
}

#[tokio::test]
async fn leaving_keeps_the_queue_and_the_chain_and_joining_again_goes_on() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), audited()).await;
    h.bao.fail_next(Failure::Status(503), 1);
    h.orgs.refresh(ACME).await.unwrap();
    h.orgs.remove(ACME).await.unwrap();
    assert_eq!(history(&h).await, vec![]);
    let kept = queued(&h).await;
    assert_eq!(kept.len(), 2);

    h.join_acme(entries(), audited()).await;
    let records = queued(&h).await;
    let rejoined = records.last().unwrap();
    assert_eq!((rejoined.seq, rejoined.event), (3, Event::Fetched));
    assert_eq!(rejoined.prev, Some(line_hash(&kept[1].line())));
}

/// Records written at once (a run's start and a fetch, say) never take one seq: the writer
/// takes them one at a time, and the chain stays whole.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn records_written_at_once_take_one_seq_each() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), audited()).await;
    let org = h.orgs.org_audit(ACME).unwrap();
    let writes: Vec<_> = (0..20)
        .map(|i| {
            let (audit, org) = (h.orgs.audit().clone(), org.clone());
            tokio::spawn(async move {
                let name = format!("NAME_{i}");
                let what = Happened {
                    event: Event::Resolved,
                    name: Some(&name),
                    consumer: Some("shell"),
                    source: None,
                    reason: None,
                };
                audit.record(&org, &Usage::default(), what).await;
            })
        })
        .collect();
    for write in writes {
        write.await.unwrap();
    }
    let seqs: Vec<u64> = queued(&h).await.iter().map(|r| r.seq).collect();
    assert_eq!(seqs, (1..=21).collect::<Vec<u64>>());
    let lines: String = h
        .store
        .audit_batch(ACME, 500)
        .await
        .unwrap()
        .iter()
        .map(|(_, line)| format!("{line}\n"))
        .collect();
    assert!(verify_lines(&lines).findings.is_empty());
}

#[test]
fn the_source_identity_is_a_login_name_a_client_id_or_the_age_public_key() {
    let token = SourceCreds::Token {
        token: Zeroizing::new("hvs.x".into()),
    };
    assert_eq!(token.identity(), None);
    let login = SourceCreds::Login {
        username: Zeroizing::new("jdoe".into()),
        password: Zeroizing::new("pw-5c1e9a".into()),
    };
    assert_eq!(login.identity(), Some("jdoe".into()));
    let machine = SourceCreds::Machine {
        client_id: Zeroizing::new("ci-7f2a".into()),
        client_secret: Zeroizing::new("cs-91b0".into()),
    };
    assert_eq!(machine.identity(), Some("ci-7f2a".into()));
    let (identity, public) = sops::new_identity();
    assert_eq!(SourceCreds::Age { identity }.identity(), Some(public));
}

/// A run start that asks the restarted service for `API_KEY` in the shell.
async fn refused_run(h: &OrgsHarness, orgs: &dots_core::vault::org::Orgs) -> Unresolved {
    let vault = Vault::new(h.store.clone(), h.secrets.clone()).with_orgs(orgs.clone());
    let granted = Grants([("shell".to_string(), vec!["API_KEY".to_string()])].into());
    vault
        .resolve_in(
            &DotScope::Org(ACME.into()),
            "default",
            &granted,
            &Consumer::Shell,
            &["API_KEY".to_string()],
            &Usage::run("01J9DOT", "Triage", "01J9RUN"),
        )
        .await
        .unwrap()
        .unwrap_err()
        .why
}

/// After a restart with no saved copy there is no policy in memory, but the last one accepted
/// had `audit`: a refused run is still queued, with the next seq, on the same chain.
#[tokio::test]
async fn after_a_restart_with_no_copy_records_still_queue_with_the_next_seq() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), audited()).await;
    std::fs::remove_file(h.copy_path(ACME)).unwrap();

    let restarted = h.service();
    restarted.load().await.unwrap();
    assert_eq!(restarted.policy(ACME), None);
    assert_eq!(refused_run(&h, &restarted).await, Unresolved::OrgOffline);

    let records = queued(&h).await;
    assert_eq!(
        records
            .iter()
            .map(|r| (r.seq, r.event, r.reason.clone()))
            .collect::<Vec<_>>(),
        vec![
            (1, Event::Fetched, None),
            (2, Event::Denied, Some("org_offline".to_string())),
        ]
    );
    let batch = h.store.audit_batch(ACME, 500).await.unwrap();
    let body: String = batch.iter().map(|(_, l)| format!("{l}
")).collect();
    assert!(verify_lines(&body).ok());
    assert_eq!(
        history(&h).await,
        vec![(
            Event::Denied,
            Some("org_offline".to_string()),
            HistoryState::Queued
        )]
    );
    let target = restarted.audit_target(ACME).expect("the last accepted target");
    assert_eq!(
        (target.endpoint.as_str(), target.token_entry.as_str()),
        ("https://audit.acme.test/dots", "DOTS_AUDIT_TOKEN")
    );
}

/// An accepted policy without `audit` forgets the target, and so does Remove: after a restart
/// with no copy, records are only listed again.
#[tokio::test]
async fn a_policy_without_audit_and_a_remove_forget_the_audit_target() {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), audited()).await;
    assert!(h.store.audit_target(ACME).await.unwrap().is_some());
    h.serve_acme(entries(), json!({}));
    h.orgs.refresh(ACME).await.unwrap();
    assert_eq!(h.store.audit_target(ACME).await.unwrap(), None);
    assert_eq!(h.orgs.audit_target(ACME), None);
    std::fs::remove_file(h.copy_path(ACME)).unwrap();

    let restarted = h.service();
    restarted.load().await.unwrap();
    assert_eq!(refused_run(&h, &restarted).await, Unresolved::OrgOffline);
    assert_eq!(queued(&h).await.len(), 1, "only the join's fetched");
    assert_eq!(
        history(&h).await[0],
        (
            Event::Denied,
            Some("org_offline".to_string()),
            HistoryState::Local
        )
    );

    h.serve_acme(entries(), audited());
    restarted.refresh(ACME).await.unwrap();
    assert!(h.store.audit_target(ACME).await.unwrap().is_some());
    restarted.remove(ACME).await.unwrap();
    assert_eq!(h.store.audit_target(ACME).await.unwrap(), None);
}
