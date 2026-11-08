//! Sending audit records (env vault spec §10, `docs/vault-audit-format.md`): an org's queue
//! goes to its endpoint with the token entry's value as the bearer, at most 500 records a
//! request; a 2xx empties what was sent, and anything else keeps it and waits, from 30 seconds
//! doubling to an hour. New records go 5 seconds after they are written.

mod common;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use common::orgs::{OrgsHarness, ACME};
use dots_core::store::HistoryState;
use dots_core::vault::audit::sender::{retry_after, AuditSender, AuditTransport};
use dots_core::vault::audit::{verify_lines, Event, Happened, Usage};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

const AUDIT_TOKEN: &str = "audit-token-value-5e92b1";
const SECRET: &str = "org-secret-api-key-90c4d1";
const ENDPOINT: &str = "https://audit.acme.test/dots";

#[derive(Debug, Clone)]
struct Post {
    endpoint: String,
    token: String,
    body: String,
}

/// Answers each post with the next scripted answer, or a 2xx when none is left.
#[derive(Debug, Default)]
struct FakeTransport {
    posts: Mutex<Vec<Post>>,
    answers: Mutex<VecDeque<Result<(), String>>>,
}

impl FakeTransport {
    fn answer(&self, answer: Result<(), String>) {
        self.answers.lock().unwrap().push_back(answer);
    }

    fn posts(&self) -> Vec<Post> {
        self.posts.lock().unwrap().clone()
    }
}

#[async_trait]
impl AuditTransport for FakeTransport {
    async fn post(&self, endpoint: &str, token: &str, body: String) -> Result<(), String> {
        self.posts.lock().unwrap().push(Post {
            endpoint: endpoint.into(),
            token: token.into(),
            body,
        });
        self.answers.lock().unwrap().pop_front().unwrap_or(Ok(()))
    }
}

fn entries() -> Value {
    json!({"DOTS_AUDIT_TOKEN": AUDIT_TOKEN, "API_KEY": "org-secret-api-key-90c4d1"})
}

fn audited() -> Value {
    json!({"audit": {"endpoint": ENDPOINT, "token_entry": "DOTS_AUDIT_TOKEN"}})
}

async fn joined(extra: Value) -> (OrgsHarness, Arc<FakeTransport>, Arc<AuditSender>) {
    let h = OrgsHarness::new().await;
    h.join_acme(entries(), extra).await;
    let transport = Arc::new(FakeTransport::default());
    let sender = AuditSender::new(h.orgs.clone(), transport.clone());
    (h, transport, sender)
}

/// `n` resolutions of `API_KEY` by the dot's shell.
async fn resolve(h: &OrgsHarness, n: usize) {
    let org = h.orgs.org_audit(ACME).unwrap();
    let usage = Usage::run("01J9DOT", "Triage", "01J9RUN");
    for _ in 0..n {
        let what = Happened {
            event: Event::Resolved,
            name: Some("API_KEY"),
            consumer: Some("shell"),
            source: None,
            reason: None,
        };
        h.orgs.audit().record(&org, &usage, what).await;
    }
}

/// No served value in any body posted, in the state shown, or in History.
async fn assert_no_value(h: &OrgsHarness, transport: &FakeTransport, sender: &AuditSender) {
    for post in transport.posts() {
        assert!(!post.body.contains(SECRET));
    }
    let shown = format!("{:?}", sender.state(ACME));
    assert!(!shown.contains(SECRET) && !shown.contains(AUDIT_TOKEN));
    let history = format!("{:?}", h.store.audit_history(ACME, 1000).await.unwrap());
    assert!(!history.contains(SECRET) && !history.contains(AUDIT_TOKEN));
}

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn the_wait_doubles_from_thirty_seconds_up_to_an_hour_and_varies_by_a_fifth() {
    let secs = |failures, jitter| retry_after(failures, jitter).num_seconds();
    assert_eq!(
        [1, 2, 3, 4, 7, 8, 30].map(|f| secs(f, 0.0)),
        [30, 60, 120, 240, 1920, 3600, 3600]
    );
    assert_eq!((secs(1, 1.0), secs(1, -1.0)), (36, 24));
    assert_eq!((secs(8, 1.0), secs(8, -1.0)), (3600, 2880));
}

#[tokio::test]
async fn a_pass_posts_the_queue_with_the_token_and_a_2xx_empties_it() {
    let (h, transport, sender) = joined(audited()).await;
    resolve(&h, 2).await;
    assert_eq!(sender.pass().await, 3);

    let posts = transport.posts();
    assert_eq!(posts.len(), 1);
    assert_eq!(posts[0].endpoint, ENDPOINT);
    assert_eq!(posts[0].token, AUDIT_TOKEN);
    assert!(!posts[0].body.contains(AUDIT_TOKEN));
    assert!(posts[0].body.ends_with('\n'));
    assert_eq!(posts[0].body.lines().count(), 3);
    let report = verify_lines(&posts[0].body);
    assert!(report.findings.is_empty());
    assert_eq!(
        (report.chains[0].first_seq, report.chains[0].last_seq),
        (1, 3)
    );

    assert_eq!(h.store.audit_queued(ACME).await.unwrap(), 0);
    let states: Vec<HistoryState> = h
        .store
        .audit_history(ACME, 100)
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.state)
        .collect();
    assert_eq!(states, vec![HistoryState::Sent, HistoryState::Sent]);
    let state = sender.state(ACME);
    assert_eq!(state.last_sent_at.as_deref(), Some("2026-10-22T09:00:00Z"));
    assert_eq!(state.last_error, None);
    // Nothing is left to send.
    assert_eq!(sender.pass().await, 0);
    assert_eq!(transport.posts().len(), 1);
    assert_no_value(&h, &transport, &sender).await;
}

#[tokio::test]
async fn more_than_five_hundred_records_go_five_hundred_at_a_time() {
    let (h, transport, sender) = joined(audited()).await;
    resolve(&h, 600).await;
    assert_eq!(sender.pass().await, 601);
    let sizes: Vec<usize> = transport
        .posts()
        .iter()
        .map(|p| p.body.lines().count())
        .collect();
    assert_eq!(sizes, vec![500, 101]);
    let all: String = transport.posts().iter().map(|p| p.body.clone()).collect();
    assert!(verify_lines(&all).findings.is_empty());
    assert_no_value(&h, &transport, &sender).await;
}

#[tokio::test]
async fn a_failed_send_keeps_the_batch_and_waits_before_the_next_try() {
    let (h, transport, sender) = joined(audited()).await;
    transport.answer(Err("the endpoint answered 503".into()));
    transport.answer(Err("the endpoint answered 503".into()));
    assert_eq!(sender.pass().await, 0);
    assert_eq!(h.store.audit_queued(ACME).await.unwrap(), 1);
    let state = sender.state(ACME);
    assert_eq!(
        state.last_error.as_deref(),
        Some("the endpoint answered 503")
    );
    assert_eq!(state.failed_at.as_deref(), Some("2026-10-22T09:00:00Z"));
    let next = at(state.next_try_at.as_deref().unwrap());
    let wait = next - h.now();
    assert!(
        wait >= Duration::seconds(24) && wait <= Duration::seconds(36),
        "{wait}"
    );

    // Not before the wait ends, even with a new record.
    resolve(&h, 1).await;
    assert_eq!(sender.pass().await, 0);
    assert_eq!(transport.posts().len(), 1);

    // The second failure waits twice as long.
    // `next_try_at` is to the second: one more passes the deadline.
    h.clock.set(next + Duration::seconds(1));
    assert_eq!(sender.pass().await, 0);
    let second = at(sender.state(ACME).next_try_at.as_deref().unwrap()) - h.now();
    assert!(
        second >= Duration::seconds(48) && second <= Duration::seconds(72),
        "{second}"
    );

    // A 2xx sends both and sets the wait back.
    h.advance(Duration::seconds(72));
    assert_eq!(sender.pass().await, 2);
    let state = sender.state(ACME);
    assert_eq!((state.last_error, state.next_try_at), (None, None));
    assert_eq!(h.store.audit_queued(ACME).await.unwrap(), 0);
    assert_no_value(&h, &transport, &sender).await;
}

#[tokio::test]
async fn without_audit_or_without_the_token_nothing_is_posted() {
    let (h, transport, sender) = joined(json!({})).await;
    assert_eq!(sender.pass().await, 0);
    assert_eq!(transport.posts().len(), 0);
    assert_eq!(h.store.audit_queued(ACME).await.unwrap(), 0);

    let h = OrgsHarness::new().await;
    h.join_acme(json!({"API_KEY": "x-value-77a1"}), audited())
        .await;
    let transport = Arc::new(FakeTransport::default());
    let sender = AuditSender::new(h.orgs.clone(), transport.clone());
    assert_eq!(sender.pass().await, 0);
    assert_eq!(transport.posts().len(), 0);
    assert_eq!(h.store.audit_queued(ACME).await.unwrap(), 1);
    assert_eq!(
        sender.state(ACME).last_error.as_deref(),
        Some("the audit token DOTS_AUDIT_TOKEN is not available: the account has no such entry")
    );
    assert_no_value(&h, &transport, &sender).await;
}

#[tokio::test]
async fn an_account_that_loses_its_target_during_a_wait_does_not_make_the_loop_spin() {
    let (h, transport, sender) = joined(audited()).await;
    transport.answer(Err("the endpoint answered 503".into()));
    let shutdown = CancellationToken::new();
    let task = sender.clone().spawn(shutdown.clone());
    // The first try fails and the loop sleeps until the retry.
    let retry = h.clock.nth_sleep(1).await;
    assert_eq!(transport.posts().len(), 1);
    assert!(sender.state(ACME).next_try_at.is_some());

    // The person removes the account; the queue stays, and the wait ends.
    h.orgs.remove(ACME).await.unwrap();
    h.clock.set(retry + Duration::seconds(1));
    tokio::time::sleep(StdDuration::from_millis(300)).await;
    let settled = h.clock.sleeps();
    assert!(settled <= 3, "{settled}");
    tokio::time::sleep(StdDuration::from_millis(300)).await;
    assert_eq!(h.clock.sleeps(), settled);
    assert_eq!(transport.posts().len(), 1);
    // Its old error and wait are gone, so they do not show after a rejoin.
    let state = sender.state(ACME);
    assert_eq!((state.last_error, state.next_try_at), (None, None));
    assert_eq!(sender.pass().await, 0);
    assert_no_value(&h, &transport, &sender).await;

    shutdown.cancel();
    tokio::time::timeout(StdDuration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn the_loop_sends_a_new_record_five_seconds_after_it_is_written() {
    let (h, transport, sender) = joined(audited()).await;
    let shutdown = CancellationToken::new();
    let task = sender.clone().spawn(shutdown.clone());
    // The first pass sends the join's record at once.
    let waited = |n: usize| {
        let transport = transport.clone();
        async move {
            for _ in 0..500 {
                if transport.posts().len() >= n {
                    return;
                }
                tokio::time::sleep(StdDuration::from_millis(10)).await;
            }
            panic!("no post {n}");
        }
    };
    waited(1).await;
    resolve(&h, 1).await;
    let deadline = h.clock.nth_sleep(1).await;
    assert_eq!(deadline, h.now() + Duration::seconds(5));
    assert_eq!(transport.posts().len(), 1);
    h.clock.set(deadline);
    waited(2).await;
    assert_eq!(transport.posts()[1].body.lines().count(), 1);
    assert_no_value(&h, &transport, &sender).await;
    shutdown.cancel();
    tokio::time::timeout(StdDuration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
}

/// After a restart with no saved copy, the account's last accepted target still holds: the
/// sender tries it, and says why the token is not there, instead of nothing.
#[tokio::test]
async fn after_a_restart_with_no_copy_the_sender_says_why_the_token_is_missing() {
    let (h, _, _) = joined(audited()).await;
    std::fs::remove_file(h.copy_path(ACME)).unwrap();
    let restarted = h.service();
    restarted.load().await.unwrap();
    let transport = Arc::new(FakeTransport::default());
    let sender = AuditSender::new(restarted.clone(), transport.clone());

    assert_eq!(sender.pass().await, 0);
    assert!(transport.posts().is_empty());
    let state = sender.state(ACME);
    assert_eq!(
        state.last_error.as_deref(),
        Some("the audit token DOTS_AUDIT_TOKEN is not available: offline, with no saved copy")
    );
    assert!(state.failed_at.is_some());
    assert_no_value(&h, &transport, &sender).await;
}
