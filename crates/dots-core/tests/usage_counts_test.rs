//! Hourly usage counts sent home (usage counts spec): off until the person turns them on with a
//! destination, plain numbers per completed UTC hour, sent once each, caught up 24 hours back.

mod common;

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use common::clock::FakeClock;
use common::{spec, temp_store};
use dots_core::model::{NewRun, RunStatus, TriggerKind};
use dots_core::store::Store;
use dots_core::usage_counts::{
    check_url, CountsSender, HttpSender, Report, UsageCounts, CATCH_UP_HOURS,
};
use serde_json::{json, Value};

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

/// A sender that records what it was given and answers as told.
#[derive(Debug, Default)]
struct FakeSender {
    sent: Mutex<Vec<(String, Value)>>,
    fail: Mutex<Option<String>>,
}

impl FakeSender {
    fn hours(&self) -> Vec<String> {
        self.sent
            .lock()
            .unwrap()
            .iter()
            .map(|(_, v)| v["hour_start"].as_str().unwrap().to_string())
            .collect()
    }
    fn fail_with(&self, e: Option<&str>) {
        *self.fail.lock().unwrap() = e.map(str::to_string);
    }
}

#[async_trait]
impl CountsSender for FakeSender {
    async fn send(&self, url: &str, report: &Report) -> Result<(), String> {
        if let Some(e) = self.fail.lock().unwrap().clone() {
            return Err(e);
        }
        self.sent
            .lock()
            .unwrap()
            .push((url.to_string(), serde_json::to_value(report).unwrap()));
        Ok(())
    }
}

const URL: &str = "https://counts.example.com/v1/hourly";

async fn service(
    now: &str,
) -> (
    tempfile::TempDir,
    Store,
    Arc<FakeClock>,
    Arc<FakeSender>,
    Arc<UsageCounts>,
) {
    let (dir, store) = temp_store().await;
    let clock = FakeClock::new(at(now));
    let sender = Arc::new(FakeSender::default());
    let counts = UsageCounts::new(store.clone(), clock.clone(), sender.clone(), "1.2.3".into());
    (dir, store, clock, sender, counts)
}

/// A run of `dot` with a status, queued, started and ended at the given times.
async fn run(
    store: &Store,
    dot: &str,
    trigger: TriggerKind,
    status: RunStatus,
    queued: &str,
    started: Option<&str>,
    ended: Option<&str>,
) -> String {
    let r = store.create_run(&NewRun::new(dot, trigger)).await.unwrap();
    store.set_status(&r.id, status).await.unwrap();
    store.set_run_queued_at(&r.id, queued).await.unwrap();
    if let Some(started) = started {
        store.set_run_times(&r.id, started, ended).await.unwrap();
    }
    r.id
}

#[tokio::test]
async fn counts_are_taken_per_hour_with_the_start_in_and_the_end_out() {
    let (_d, store, _c, _s, counts) = service("2026-11-05T12:30:00.000Z").await;
    let dot = store.create_dot(&spec("a"), None, "default").await.unwrap();
    store.create_dot(&spec("b"), None, "default").await.unwrap();
    let h = "2026-11-05T10:00:00.000Z";
    // Started on the hour's first millisecond, ended in it: started and succeeded.
    run(
        &store,
        &dot.id,
        TriggerKind::Schedule,
        RunStatus::Succeeded,
        h,
        Some(h),
        Some("2026-11-05T10:20:00.000Z"),
    )
    .await;
    // Started the hour before, failed in this hour: failed only.
    run(
        &store,
        &dot.id,
        TriggerKind::Manual,
        RunStatus::Failed,
        "2026-11-05T09:58:00.000Z",
        Some("2026-11-05T09:59:59.999Z"),
        Some("2026-11-05T10:01:00.000Z"),
    )
    .await;
    // Started on the next hour's first millisecond: not this hour.
    run(
        &store,
        &dot.id,
        TriggerKind::Schedule,
        RunStatus::Succeeded,
        "2026-11-05T11:00:00.000Z",
        Some("2026-11-05T11:00:00.000Z"),
        Some("2026-11-05T11:05:00.000Z"),
    )
    .await;
    // Started in the hour, still running at its end: started only.
    run(
        &store,
        &dot.id,
        TriggerKind::Schedule,
        RunStatus::Running,
        "2026-11-05T10:59:00.000Z",
        Some("2026-11-05T10:59:59.999Z"),
        None,
    )
    .await;
    // Cancelled in the hour: started, neither succeeded nor failed.
    run(
        &store,
        &dot.id,
        TriggerKind::Schedule,
        RunStatus::Cancelled,
        "2026-11-05T10:10:00.000Z",
        Some("2026-11-05T10:10:00.000Z"),
        Some("2026-11-05T10:11:00.000Z"),
    )
    .await;
    // Two chat messages sent in the hour (one waiting), one at the next hour, one before.
    let chat = run(
        &store,
        &dot.id,
        TriggerKind::Chat,
        RunStatus::Succeeded,
        "2026-11-05T10:30:00.000Z",
        Some("2026-11-05T10:30:01.000Z"),
        Some("2026-11-05T10:31:00.000Z"),
    )
    .await;
    run(
        &store,
        &dot.id,
        TriggerKind::Chat,
        RunStatus::Queued,
        "2026-11-05T10:59:59.999Z",
        None,
        None,
    )
    .await;
    run(
        &store,
        &dot.id,
        TriggerKind::Chat,
        RunStatus::Queued,
        "2026-11-05T11:00:00.000Z",
        None,
        None,
    )
    .await;
    run(
        &store,
        &dot.id,
        TriggerKind::Chat,
        RunStatus::Queued,
        "2026-11-05T09:59:59.999Z",
        None,
        None,
    )
    .await;
    // Approvals asked: one in the hour, one on the next hour's start.
    let a = store
        .create_approval(&chat, "Bash", &json!({"command": "ls"}))
        .await
        .unwrap();
    store
        .set_approval_created_at(&a.id, "2026-11-05T10:45:00.000Z")
        .await
        .unwrap();
    let b = store
        .create_approval(&chat, "Bash", &json!({"command": "dir"}))
        .await
        .unwrap();
    store
        .set_approval_created_at(&b.id, "2026-11-05T11:00:00.000Z")
        .await
        .unwrap();

    let c = store.hour_counts(at(h)).await.unwrap();
    assert_eq!(c.runs_started, 4, "{c:?}");
    assert_eq!(c.runs_succeeded, 2, "{c:?}"); // the schedule run and the chat run
    assert_eq!(c.runs_failed, 1, "{c:?}");
    assert_eq!(c.messages_sent, 2, "{c:?}");
    assert_eq!(c.approvals_asked, 1, "{c:?}");
    assert_eq!(c.dots, 2, "{c:?}");
    let _ = counts;
}

#[tokio::test]
async fn a_report_holds_only_the_allowed_keys_and_numbers() {
    let (_d, store, _c, sender, counts) = service("2026-11-05T09:30:00.000Z").await;
    store
        .create_dot(&spec("secret-project"), None, "default")
        .await
        .unwrap();
    counts.set(true, Some(URL.into())).await.unwrap();
    let clock_now = at("2026-11-05T11:05:00.000Z");
    _c.set(clock_now);
    assert_eq!(counts.pass().await.unwrap(), 1);
    let sent = sender.sent.lock().unwrap().clone();
    let (url, body) = &sent[0];
    assert_eq!(url, URL);
    let keys: BTreeSet<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    let allowed: BTreeSet<&str> = [
        "schema",
        "install_id",
        "app_version",
        "hour_start",
        "runs_started",
        "runs_succeeded",
        "runs_failed",
        "messages_sent",
        "approvals_asked",
        "dots",
    ]
    .into_iter()
    .collect();
    assert_eq!(keys, allowed);
    assert_eq!(body["schema"], 1);
    assert_eq!(body["app_version"], "1.2.3");
    assert_eq!(body["hour_start"], "2026-11-05T10:00:00Z");
    assert_eq!(body["dots"], 1);
    let id = body["install_id"].as_str().unwrap();
    assert_eq!(id.len(), 36);
    assert_eq!(&id[14..15], "4", "a version 4 UUID: {id}");
    for key in &allowed {
        if !matches!(*key, "install_id" | "app_version" | "hour_start") {
            assert!(body[key].is_u64(), "{key} is a number: {body}");
        }
    }
    assert!(!body.to_string().contains("secret-project"));
}

#[tokio::test]
async fn usage_counts_are_off_by_default_and_nothing_is_sent() {
    let (_d, _store, clock, sender, counts) = service("2026-11-05T09:30:00.000Z").await;
    let state = counts.state().await.unwrap();
    assert!(!state.enabled);
    assert_eq!(state.url, None);
    assert_eq!(state.install_id, None);
    clock.set(at("2026-11-06T09:30:00.000Z"));
    assert_eq!(counts.pass().await.unwrap(), 0);
    assert!(sender.hours().is_empty());
}

#[tokio::test]
async fn they_cannot_be_turned_on_without_a_valid_destination() {
    let (_d, _store, _clock, _sender, counts) = service("2026-11-05T09:30:00.000Z").await;
    assert!(counts.set(true, None).await.is_err());
    assert!(counts.set(true, Some("  ".into())).await.is_err());
    assert!(counts
        .set(true, Some("http://counts.example.com/".into()))
        .await
        .is_err());
    assert!(!counts.state().await.unwrap().enabled);
    // A destination can be kept while off, and turned on later.
    let state = counts.set(false, Some(URL.into())).await.unwrap();
    assert!(!state.enabled);
    assert_eq!(state.url.as_deref(), Some(URL));
    assert!(counts.set(true, None).await.unwrap().enabled);
    let state = counts.state().await.unwrap();
    assert!(state.install_id.is_some());
}

#[test]
fn destinations_must_be_https_except_this_pc() {
    for ok in [
        "https://counts.example.com/v1",
        "https://counts.example.com:8443/hourly?x=1",
        "http://localhost:8080/counts",
        "http://127.0.0.1:9000/",
        "http://localhost/",
    ] {
        assert!(check_url(ok).is_ok(), "{ok}");
    }
    for bad in [
        "",
        "counts.example.com",
        "http://counts.example.com/",
        "http://192.168.1.5/",
        "http://localhost.example.com/",
        "ftp://counts.example.com/",
        "file:///C:/counts.json",
        "https://user:pass@counts.example.com/",
        "https://",
        "javascript:alert(1)",
    ] {
        assert!(check_url(bad).is_err(), "{bad}");
    }
}

#[tokio::test]
async fn each_completed_hour_after_turning_on_is_sent_once() {
    let (_d, _store, clock, sender, counts) = service("2026-11-05T09:30:00.000Z").await;
    counts.set(true, Some(URL.into())).await.unwrap();
    // The hour it was turned on in is not sent: the first is the next whole hour.
    clock.set(at("2026-11-05T10:01:00.000Z"));
    assert_eq!(counts.pass().await.unwrap(), 0);
    clock.set(at("2026-11-05T11:00:30.000Z"));
    assert_eq!(counts.pass().await.unwrap(), 1);
    assert_eq!(counts.pass().await.unwrap(), 0, "an hour is sent once");
    clock.set(at("2026-11-05T12:02:00.000Z"));
    assert_eq!(counts.pass().await.unwrap(), 1);
    assert_eq!(
        sender.hours(),
        ["2026-11-05T10:00:00Z", "2026-11-05T11:00:00Z"]
    );
    let state = counts.state().await.unwrap();
    assert_eq!(state.last_sent_hour, Some(at("2026-11-05T11:00:00Z")));
    assert_eq!(state.last_sent_at, Some(at("2026-11-05T12:02:00Z")));
    assert_eq!(state.last_error, None);
}

#[tokio::test]
async fn missed_hours_are_caught_up_oldest_first_and_older_than_a_day_dropped() {
    let (_d, _store, clock, sender, counts) = service("2026-11-05T09:30:00.000Z").await;
    counts.set(true, Some(URL.into())).await.unwrap();
    // Closed for 30 hours: hours 10:00 on the 5th to 15:00 on the 6th are complete.
    clock.set(at("2026-11-06T16:10:00.000Z"));
    assert_eq!(counts.pass().await.unwrap(), CATCH_UP_HOURS as usize);
    let hours = sender.hours();
    assert_eq!(hours.len(), 24);
    assert_eq!(hours[0], "2026-11-05T16:00:00Z");
    assert_eq!(hours[23], "2026-11-06T15:00:00Z");
    let mut sorted = hours.clone();
    sorted.sort();
    assert_eq!(hours, sorted, "oldest first");
}

#[tokio::test]
async fn a_failed_send_keeps_the_hour_for_the_next_try() {
    let (_d, _store, clock, sender, counts) = service("2026-11-05T09:30:00.000Z").await;
    counts.set(true, Some(URL.into())).await.unwrap();
    clock.set(at("2026-11-05T12:01:00.000Z"));
    sender.fail_with(Some("the server answered 503"));
    assert_eq!(counts.pass().await.unwrap(), 0);
    let state = counts.state().await.unwrap();
    assert_eq!(state.last_error.as_deref(), Some("the server answered 503"));
    assert_eq!(state.last_sent_hour, None);
    sender.fail_with(None);
    clock.set(at("2026-11-05T12:11:00.000Z"));
    assert_eq!(counts.pass().await.unwrap(), 2);
    assert_eq!(
        sender.hours(),
        ["2026-11-05T10:00:00Z", "2026-11-05T11:00:00Z"]
    );
    assert_eq!(counts.state().await.unwrap().last_error, None);
}

#[tokio::test]
async fn turning_off_clears_the_queue_and_stops_sending() {
    let (_d, _store, clock, sender, counts) = service("2026-11-05T09:30:00.000Z").await;
    counts.set(true, Some(URL.into())).await.unwrap();
    clock.set(at("2026-11-05T13:01:00.000Z"));
    sender.fail_with(Some("no answer"));
    counts.pass().await.unwrap();
    let off = counts.set(false, None).await.unwrap();
    assert!(!off.enabled);
    assert_eq!(off.last_error, None);
    assert_eq!(
        off.url.as_deref(),
        Some(URL),
        "the destination is kept for next time"
    );
    sender.fail_with(None);
    assert_eq!(counts.pass().await.unwrap(), 0);
    // Turned on again, the hours while it was off are never sent.
    counts.set(true, None).await.unwrap();
    clock.set(at("2026-11-05T15:01:00.000Z"));
    assert_eq!(counts.pass().await.unwrap(), 1);
    assert_eq!(sender.hours(), ["2026-11-05T14:00:00Z"]);
}

#[tokio::test]
async fn reset_id_makes_a_new_install_id() {
    let (_d, _store, _clock, _sender, counts) = service("2026-11-05T09:30:00.000Z").await;
    assert_eq!(
        counts.reset_id().await.unwrap().install_id,
        None,
        "none before turned on"
    );
    let first = counts
        .set(true, Some(URL.into()))
        .await
        .unwrap()
        .install_id
        .unwrap();
    let second = counts.reset_id().await.unwrap().install_id.unwrap();
    assert_ne!(first, second);
    // Turning off and on keeps the id.
    counts.set(false, None).await.unwrap();
    let again = counts.set(true, None).await.unwrap().install_id.unwrap();
    assert_eq!(again, second);
}

#[tokio::test]
async fn the_preview_is_the_next_report_exactly() {
    let (_d, store, clock, sender, counts) = service("2026-11-05T09:30:00.000Z").await;
    store.create_dot(&spec("a"), None, "default").await.unwrap();
    let preview = serde_json::to_value(counts.preview().await.unwrap()).unwrap();
    assert_eq!(
        preview["install_id"],
        Value::Null,
        "no id before it is turned on"
    );
    assert_eq!(preview["hour_start"], "2026-11-05T10:00:00Z");
    counts.set(true, Some(URL.into())).await.unwrap();
    clock.set(at("2026-11-05T11:20:00.000Z"));
    let preview = serde_json::to_value(counts.preview().await.unwrap()).unwrap();
    counts.pass().await.unwrap();
    assert_eq!(sender.sent.lock().unwrap()[0].1, preview);
}

#[tokio::test]
async fn the_loop_sends_shortly_after_each_hour_ends() {
    let (_d, _store, clock, sender, counts) = service("2026-11-05T09:30:00.000Z").await;
    counts.set(true, Some(URL.into())).await.unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let task = counts.clone().spawn(shutdown.clone());
    let wake = clock.nth_sleep(1).await;
    assert!(
        wake > at("2026-11-05T10:00:00Z") && wake <= at("2026-11-05T10:10:00Z"),
        "{wake}"
    );
    let wake2 = {
        clock.set(wake);
        clock.nth_sleep(2).await
    };
    assert!(
        sender.hours().is_empty(),
        "the first whole hour has not ended"
    );
    clock.set(wake2);
    let _ = clock.nth_sleep(3).await;
    assert_eq!(sender.hours(), ["2026-11-05T10:00:00Z"]);
    shutdown.cancel();
    task.await.unwrap();
}

#[tokio::test]
async fn the_http_sender_posts_the_json_and_reports_a_bad_answer() {
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::start().await;
    let report = Report {
        schema: 1,
        install_id: Some("00000000-0000-4000-8000-000000000000".into()),
        app_version: "1.2.3".into(),
        hour_start: at("2026-11-05T10:00:00Z"),
        runs_started: 3,
        runs_succeeded: 2,
        runs_failed: 1,
        messages_sent: 4,
        approvals_asked: 0,
        dots: 2,
    };
    Mock::given(method("POST"))
        .and(path("/ok"))
        .and(body_json(serde_json::to_value(&report).unwrap()))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/down"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/slow"))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(5)))
        .mount(&server)
        .await;
    let sender = HttpSender::new(std::time::Duration::from_millis(500));
    let base = server.uri();
    sender.send(&format!("{base}/ok"), &report).await.unwrap();
    let e = sender
        .send(&format!("{base}/down"), &report)
        .await
        .unwrap_err();
    assert!(e.contains("503"), "{e}");
    let e = sender
        .send(&format!("{base}/slow"), &report)
        .await
        .unwrap_err();
    assert!(e.contains("no answer"), "{e}");
}
