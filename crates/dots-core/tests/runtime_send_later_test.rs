//! Sending a scheduled message (send later spec sections 4.2 to 4.5), on a `FakeClock`: the
//! target read at send time (a reply after the topic's end, a wait while the topic is busy or
//! Claude's limit is reached, a new topic when the old one cannot go on), the races with Edit,
//! Cancel and Send now, recovery after a crash, and the sender loop. The queue is held before
//! any run is created, so no dispatcher claims one mid-test.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::{spec, wait_status};
use dots_core::events::RuntimeEvent;
use dots_core::model::{
    Dot, DotSpec, NewRun, Run, RunStatus, ScheduledMessage, ScheduledStatus, ScheduledWait,
    TriggerKind, WorkspaceMode,
};
use dots_core::send_later::{SendTarget, ALREADY_SENT, BEING_SENT, TOO_FAR, TOO_SOON};
use dots_core::util::timestamp;
use dots_core::{Config, Error, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-04T09:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn at(offset: TimeDelta) -> String {
    timestamp(t0() + offset)
}

fn hour() -> TimeDelta {
    TimeDelta::hours(1)
}

fn config(data: &Path, clock: &Arc<FakeClock>) -> Config {
    let mut cfg = Config::new(data.to_path_buf());
    cfg.port = 0;
    cfg.clock = clock.clone();
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg
}

/// A runtime on `data` at `now`, with its queue held.
async fn runtime_at(data: &Path, now: DateTime<Utc>) -> (Runtime, Arc<FakeClock>) {
    let clock = FakeClock::new(now);
    let rt = Runtime::start(config(data, &clock)).await.unwrap();
    rt.hold_runs(true).await;
    (rt, clock)
}

async fn start(dir: &TempDir) -> (Runtime, Arc<FakeClock>) {
    runtime_at(&dir.path().join("data"), t0()).await
}

fn folder_spec(name: &str, dir: &TempDir) -> DotSpec {
    let mut s: DotSpec = spec(name);
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s
}

async fn dot(rt: &Runtime, dir: &TempDir) -> Dot {
    rt.create_dot(folder_spec("courier", dir), None, None)
        .await
        .unwrap()
}

/// Ends the queued run `run`: succeeded with `session`, or failed before the dot started when
/// `session` is `None`.
async fn end(rt: &Runtime, run: &Run, session: Option<&str>) -> Run {
    let store = rt.store();
    assert!(store.claim_run(&run.id).await.unwrap());
    match session {
        Some(s) => {
            store.mark_delivered(&run.id, Utc::now()).await.unwrap();
            store.set_session(&run.id, s).await.unwrap();
            store
                .finish_run(&run.id, RunStatus::Succeeded, Some("done"), None)
                .await
                .unwrap()
        }
        None => store
            .finish_run(
                &run.id,
                RunStatus::Failed,
                None,
                Some("claude CLI not found"),
            )
            .await
            .unwrap(),
    }
}

/// A topic: the person's first message, answered with session `s1`.
async fn topic(rt: &Runtime, dot: &Dot) -> Run {
    let p = rt.chat(&dot.id, "Hello", None).await.unwrap();
    end(rt, &p, Some("s1")).await
}

async fn schedule(rt: &Runtime, dot: &Dot, root: Option<&Run>, due: TimeDelta) -> ScheduledMessage {
    rt.schedule_message(&dot.id, "Ship it", root.map(|r| r.id.as_str()), &at(due))
        .await
        .unwrap()
}

async fn row(rt: &Runtime, id: &str) -> ScheduledMessage {
    rt.store().get_scheduled_message(id).await.unwrap()
}

async fn sent_run(rt: &Runtime, message: &ScheduledMessage) -> Run {
    let message = row(rt, &message.id).await;
    assert_eq!(message.status, ScheduledStatus::Sent, "{message:?}");
    rt.get_run(message.run_id.as_deref().unwrap())
        .await
        .unwrap()
}

fn note(run: &Run) -> Value {
    run.payload.as_ref().unwrap()["send_later"].clone()
}

async fn runs_of(rt: &Runtime, dot: &Dot) -> usize {
    rt.store()
        .list_runs(Some(&dot.id), 100)
        .await
        .unwrap()
        .len()
}

/// The `ScheduledMessageUpdated` rows sent so far.
fn updates(rx: &mut Receiver<RuntimeEvent>) -> Vec<ScheduledMessage> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::ScheduledMessageUpdated { message } = ev {
            out.push(message);
        }
    }
    out
}

fn conflict(e: Error) -> String {
    match e {
        Error::Conflict(m) => m,
        other => panic!("expected Conflict, got {other:?}"),
    }
}

fn invalid(e: Error) -> String {
    match e {
        Error::Invalid(m) => m,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[tokio::test]
async fn scheduling_validates_and_lists() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, _clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let other = rt
        .create_dot(folder_spec("other", &dir), None, None)
        .await
        .unwrap();
    let foreign = rt.chat(&other.id, "Hi", None).await.unwrap();
    let mut rx = rt.subscribe();

    let refused = |due: TimeDelta, text: &'static str| {
        let rt = &rt;
        let dot = &dot;
        async move {
            let e = rt
                .schedule_message(&dot.id, text, None, &at(due))
                .await
                .unwrap_err();
            invalid(e)
        }
    };
    assert_eq!(refused(TimeDelta::seconds(30), "Ship it").await, TOO_SOON);
    assert_eq!(refused(-hour(), "Ship it").await, TOO_SOON);
    assert_eq!(refused(TimeDelta::days(366), "Ship it").await, TOO_FAR);
    assert_eq!(refused(hour(), "  ").await, "message must not be empty");
    let e = rt
        .schedule_message(&dot.id, "Ship it", Some(&foreign.id), &at(hour()))
        .await
        .unwrap_err();
    assert!(invalid(e).contains("belongs to a different dot"));
    assert!(updates(&mut rx).is_empty());

    let message = schedule(&rt, &dot, None, hour()).await;
    assert_eq!(message.status, ScheduledStatus::Scheduled);
    assert_eq!(message.due_at, at(hour()));
    assert_eq!(
        rt.scheduled_messages(&dot.id).await.unwrap(),
        [message.clone()]
    );
    assert_eq!(updates(&mut rx), [message]);
    rt.shutdown();
}

#[tokio::test]
async fn a_due_message_replies_to_its_topic() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let p = topic(&rt, &dot).await;
    let message = schedule(&rt, &dot, Some(&p), hour()).await;

    // Not due yet: nothing goes.
    assert!(rt.send_due().await.unwrap().is_empty());
    assert_eq!(
        row(&rt, &message.id).await.status,
        ScheduledStatus::Scheduled
    );

    clock.set(t0() + hour());
    rt.send_due().await.unwrap();
    let r = sent_run(&rt, &message).await;
    assert_eq!(r.trigger, TriggerKind::Chat);
    assert_eq!(r.parent_run_id.as_deref(), Some(p.id.as_str()));
    assert_eq!(r.session_id.as_deref(), Some("s1"));
    assert_eq!(r.payload.as_ref().unwrap()["message"], "Ship it");
    assert_eq!(r.payload.as_ref().unwrap()["typed"], true);
    assert_eq!(
        note(&r),
        json!({ "id": message.id, "due_at": at(hour()), "late": false, "new_topic": false })
    );
    let sent = row(&rt, &message.id).await;
    assert_eq!(sent.sent_at, Some(at(hour())));
    assert!(rt.scheduled_messages(&dot.id).await.unwrap().is_empty());
    rt.shutdown();
}

#[tokio::test]
async fn a_message_with_no_topic_starts_one() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let message = schedule(&rt, &dot, None, hour()).await;
    clock.set(t0() + hour());
    rt.send_due().await.unwrap();
    let r = sent_run(&rt, &message).await;
    assert_eq!(r.parent_run_id, None);
    assert_eq!(note(&r)["new_topic"], false);
    rt.shutdown();
}

#[tokio::test]
async fn a_busy_topic_waits_then_replies_to_its_newest_run() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let p = topic(&rt, &dot).await;
    let message = schedule(&rt, &dot, Some(&p), hour()).await;
    // The person typed into the topic meanwhile; the dot has not answered yet.
    let q = rt.chat(&dot.id, "And then?", Some(&p.id)).await.unwrap();
    let mut rx = rt.subscribe();

    clock.set(t0() + hour());
    rt.send_due().await.unwrap();
    let waiting = row(&rt, &message.id).await;
    assert_eq!(waiting.status, ScheduledStatus::Scheduled);
    assert_eq!(waiting.waiting, Some(ScheduledWait::Busy));
    assert_eq!(runs_of(&rt, &dot).await, 2);
    assert_eq!(updates(&mut rx), [waiting]);

    // The same reason again: nothing is sent, not even an event.
    rt.send_due().await.unwrap();
    assert!(updates(&mut rx).is_empty());

    end(&rt, &q, Some("s1")).await;
    clock.set(t0() + hour() + TimeDelta::seconds(30));
    rt.send_due().await.unwrap();
    let r = sent_run(&rt, &message).await;
    assert_eq!(r.parent_run_id.as_deref(), Some(q.id.as_str()));
    assert_eq!(row(&rt, &message.id).await.waiting, None);
    rt.shutdown();
}

#[tokio::test]
async fn an_approval_in_the_topic_waits_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let p = topic(&rt, &dot).await;
    let message = schedule(&rt, &dot, Some(&p), hour()).await;
    let q = rt.chat(&dot.id, "Deploy", Some(&p.id)).await.unwrap();
    rt.store()
        .set_status(&q.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();

    clock.set(t0() + hour());
    rt.send_due().await.unwrap();
    assert_eq!(
        row(&rt, &message.id).await.waiting,
        Some(ScheduledWait::Approval)
    );
    rt.shutdown();
}

#[tokio::test]
async fn a_lost_race_in_the_chat_path_waits() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let p = topic(&rt, &dot).await;
    let message = schedule(&rt, &dot, Some(&p), hour()).await;
    // The thread moved on after the target was read: the target is stale.
    let q = rt.chat(&dot.id, "And then?", Some(&p.id)).await.unwrap();
    end(&rt, &q, Some("s1")).await;

    clock.set(t0() + hour());
    let out = rt
        .send_scheduled_to(
            row(&rt, &message.id).await,
            SendTarget::Reply {
                parent_run_id: p.id.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(out.status, ScheduledStatus::Scheduled);
    assert_eq!(out.waiting, Some(ScheduledWait::Busy));
    assert_eq!(runs_of(&rt, &dot).await, 2);

    // The next pass reads the thread again and replies to its end.
    rt.send_due().await.unwrap();
    let r = sent_run(&rt, &message).await;
    assert_eq!(r.parent_run_id.as_deref(), Some(q.id.as_str()));
    rt.shutdown();
}

#[tokio::test]
async fn a_topic_that_cannot_continue_gets_a_new_topic() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    // Its only run failed before the dot had a session.
    let p = rt.chat(&dot.id, "Hello", None).await.unwrap();
    let p = end(&rt, &p, None).await;
    let no_session = schedule(&rt, &dot, Some(&p), hour()).await;
    // Its root is gone.
    let gone = rt
        .store()
        .insert_scheduled_message(&dot.id, Some("01NOSUCHRUN"), "Ship it", &at(hour()), t0())
        .await
        .unwrap();

    clock.set(t0() + hour());
    rt.send_due().await.unwrap();
    for message in [&no_session, &gone] {
        let r = sent_run(&rt, message).await;
        assert_eq!(r.parent_run_id, None);
        assert_eq!(r.root_run_id, r.id);
        assert_eq!(note(&r)["new_topic"], true);
    }
    rt.shutdown();
}

#[tokio::test]
async fn the_usage_limit_waits() {
    let dir = tempfile::tempdir().unwrap();
    let clock = FakeClock::new(t0());
    let script = dir.path().join("script.json");
    let resets = (t0() + TimeDelta::hours(2)).timestamp();
    std::fs::write(
        &script,
        json!([
            {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
            {"emit": {"type": "rate_limit_event", "rate_limit_info": {
                "status": "rejected", "resetsAt": resets, "rateLimitType": "five_hour",
                "unifiedWindows": {"five_hour": {"utilization": 1.0, "resetsAt": resets, "status": "rejected"}}
            }}},
            {"emit": {"type": "result", "subtype": "success", "is_error": false,
                "result": "done", "usage": {"input_tokens": 1, "output_tokens": 1}}}
        ])
        .to_string(),
    )
    .unwrap();
    let mut cfg = config(&dir.path().join("data"), &clock);
    cfg.claude_env.push((
        "FAKE_CLAUDE_SCRIPT".into(),
        script.to_string_lossy().to_string(),
    ));
    let rt = Runtime::start(cfg).await.unwrap();
    // A run of another dot brings Claude's reading: rejected until T0+2h.
    let reader = rt
        .create_dot(folder_spec("reader", &dir), None, None)
        .await
        .unwrap();
    let read = rt.run_now(&reader.id).await.unwrap();
    wait_status(rt.store(), &read.id, RunStatus::Succeeded, 15).await;
    rt.hold_runs(true).await;
    assert!(rt.limits_get().await.unwrap().rejected_until.is_some());

    let dot = dot(&rt, &dir).await;
    let message = schedule(&rt, &dot, None, hour()).await;
    clock.set(t0() + hour());
    rt.send_due().await.unwrap();
    let waiting = row(&rt, &message.id).await;
    assert_eq!(waiting.waiting, Some(ScheduledWait::UsageLimit));
    assert_eq!(runs_of(&rt, &dot).await, 0);

    clock.set(t0() + TimeDelta::hours(2) + TimeDelta::minutes(1));
    rt.send_due().await.unwrap();
    let r = sent_run(&rt, &message).await;
    assert_eq!(note(&r)["late"], true);
    rt.shutdown();
}

#[tokio::test]
async fn a_refused_dot_fails_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    // A dot of an org this PC is no longer joined to.
    let dot = rt
        .store()
        .create_dot(&folder_spec("triage", &dir), Some("acme"), "default")
        .await
        .unwrap();
    let message = rt
        .store()
        .insert_scheduled_message(&dot.id, None, "Ship it", &at(hour()), t0())
        .await
        .unwrap();

    clock.set(t0() + hour());
    rt.send_due().await.unwrap();
    let failed = row(&rt, &message.id).await;
    assert_eq!(failed.status, ScheduledStatus::Failed);
    assert_eq!(
        failed.error.as_deref(),
        Some("dot 'triage': its org 'acme' was removed")
    );
    assert_eq!(runs_of(&rt, &dot).await, 0);
    rt.shutdown();
}

#[tokio::test]
async fn late_is_more_than_a_minute() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let on_time = schedule(&rt, &dot, None, hour()).await;
    clock.set(t0() + hour() + TimeDelta::seconds(30));
    rt.send_due().await.unwrap();
    assert_eq!(note(&sent_run(&rt, &on_time).await)["late"], false);

    let late = schedule(&rt, &dot, None, TimeDelta::hours(2)).await;
    clock.set(t0() + TimeDelta::hours(2) + TimeDelta::minutes(2));
    rt.send_due().await.unwrap();
    assert_eq!(note(&sent_run(&rt, &late).await)["late"], true);
    rt.shutdown();
}

#[tokio::test]
async fn two_due_for_one_topic_go_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let p = topic(&rt, &dot).await;
    let a = schedule(&rt, &dot, Some(&p), hour()).await;
    let b = schedule(&rt, &dot, Some(&p), hour() + TimeDelta::minutes(1)).await;

    // Dots was closed over both.
    clock.set(t0() + TimeDelta::hours(3));
    rt.send_due().await.unwrap();
    let ra = sent_run(&rt, &a).await;
    assert_eq!(ra.parent_run_id.as_deref(), Some(p.id.as_str()));
    assert_eq!(note(&ra)["late"], true);
    assert_eq!(row(&rt, &b.id).await.waiting, Some(ScheduledWait::Busy));

    end(&rt, &ra, Some("s1")).await;
    rt.send_due().await.unwrap();
    let rb = sent_run(&rt, &b).await;
    assert_eq!(rb.parent_run_id.as_deref(), Some(ra.id.as_str()));
    assert_eq!(runs_of(&rt, &dot).await, 3);
    rt.shutdown();
}

#[tokio::test]
async fn edit_and_cancel_lose_to_a_claim() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let message = schedule(&rt, &dot, None, hour()).await;
    clock.set(t0() + hour());
    rt.store()
        .claim_scheduled_message(&message.id, clock_now(&clock))
        .await
        .unwrap()
        .unwrap();

    let later = at(TimeDelta::hours(3));
    let e = rt
        .edit_scheduled_message(&message.id, "Other", &later)
        .await
        .unwrap_err();
    assert_eq!(conflict(e), BEING_SENT);
    let e = rt.cancel_scheduled_message(&message.id).await.unwrap_err();
    assert_eq!(conflict(e), BEING_SENT);

    let run = rt.chat(&dot.id, "Ship it", None).await.unwrap();
    rt.store()
        .mark_scheduled_sent(&message.id, &run.id, clock_now(&clock))
        .await
        .unwrap();
    let e = rt
        .edit_scheduled_message(&message.id, "Other", &later)
        .await
        .unwrap_err();
    assert_eq!(conflict(e), ALREADY_SENT);
    let e = rt.cancel_scheduled_message(&message.id).await.unwrap_err();
    assert_eq!(conflict(e), ALREADY_SENT);
    assert_eq!(row(&rt, &message.id).await.text, "Ship it");
    rt.shutdown();
}

fn clock_now(clock: &FakeClock) -> DateTime<Utc> {
    use dots_core::scheduler::Clock;
    clock.now()
}

#[tokio::test]
async fn edit_moves_the_time_and_cancel_keeps_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let message = schedule(&rt, &dot, None, hour()).await;
    let edited = rt
        .edit_scheduled_message(&message.id, "Ship it today", &at(TimeDelta::hours(2)))
        .await
        .unwrap();
    assert_eq!(edited.text, "Ship it today");
    assert_eq!(edited.due_at, at(TimeDelta::hours(2)));
    let e = rt
        .edit_scheduled_message(&message.id, "x", &at(TimeDelta::seconds(10)))
        .await
        .unwrap_err();
    assert_eq!(invalid(e), TOO_SOON);

    // Not due at its old time any more.
    clock.set(t0() + hour());
    rt.send_due().await.unwrap();
    assert_eq!(runs_of(&rt, &dot).await, 0);

    let cancelled = rt.cancel_scheduled_message(&message.id).await.unwrap();
    assert_eq!(cancelled.status, ScheduledStatus::Cancelled);
    assert_eq!(
        row(&rt, &message.id).await.status,
        ScheduledStatus::Cancelled
    );
    let e = rt.cancel_scheduled_message(&message.id).await.unwrap_err();
    assert_eq!(conflict(e), "This message was cancelled.");
    rt.shutdown();
}

#[tokio::test]
async fn the_sender_skips_a_cancelled_message() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let message = schedule(&rt, &dot, None, hour()).await;
    rt.cancel_scheduled_message(&message.id).await.unwrap();
    clock.set(t0() + TimeDelta::hours(2));
    assert!(rt.send_due().await.unwrap().is_empty());
    // A stale read of the row loses its claim too.
    let out = rt
        .send_scheduled_to(message.clone(), SendTarget::New { fallback: false })
        .await
        .unwrap();
    assert_eq!(out.status, ScheduledStatus::Cancelled);
    assert_eq!(runs_of(&rt, &dot).await, 0);
    rt.shutdown();
}

#[tokio::test]
async fn send_now_sends_at_once_and_is_not_late() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, _clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let message = schedule(&rt, &dot, None, TimeDelta::days(1)).await;

    let sent = rt.send_scheduled_now(&message.id).await.unwrap();
    assert_eq!(sent.status, ScheduledStatus::Sent);
    assert_eq!(sent.due_at, timestamp(t0()));
    assert_eq!(note(&sent_run(&rt, &message).await)["late"], false);
    let e = rt.send_scheduled_now(&message.id).await.unwrap_err();
    assert_eq!(conflict(e), ALREADY_SENT);

    // A failed message of a dot that may run again.
    let failed = rt
        .store()
        .insert_scheduled_message(&dot.id, None, "Again", &at(hour()), t0())
        .await
        .unwrap();
    rt.store()
        .mark_scheduled_failed(&failed.id, "org removed", t0())
        .await
        .unwrap();
    let sent = rt.send_scheduled_now(&failed.id).await.unwrap();
    assert_eq!(sent.status, ScheduledStatus::Sent);
    assert_eq!(sent.error, None);
    assert_eq!(runs_of(&rt, &dot).await, 2);
    rt.shutdown();
}

#[tokio::test]
async fn a_crash_while_sending_never_sends_twice() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let due = t0() + hour();

    // Claimed, and the crash came before its run was stored.
    let (rt, clock) = runtime_at(&data, t0()).await;
    let dot = dot(&rt, &dir).await;
    let lost = schedule(&rt, &dot, None, hour()).await;
    clock.set(due);
    rt.store()
        .claim_scheduled_message(&lost.id, due)
        .await
        .unwrap()
        .unwrap();
    // Claimed, and the crash came after its run was stored.
    let stored = rt
        .store()
        .insert_scheduled_message(&dot.id, None, "Stored", &at(TimeDelta::zero()), t0())
        .await
        .unwrap();
    rt.store()
        .claim_scheduled_message(&stored.id, due)
        .await
        .unwrap()
        .unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
    new.payload = Some(json!({
        "message": "Stored", "typed": true,
        "send_later": { "id": stored.id, "due_at": stored.due_at, "late": true, "new_topic": false },
    }));
    let run = rt.store().create_run(&new).await.unwrap();
    end(&rt, &run, Some("s1")).await;
    rt.shutdown();
    drop(rt);

    let (rt, _clock) = runtime_at(&data, due).await;
    let lost = row(&rt, &lost.id).await;
    assert_eq!(lost.status, ScheduledStatus::Scheduled);
    let stored = row(&rt, &stored.id).await;
    assert_eq!(stored.status, ScheduledStatus::Sent);
    assert_eq!(stored.run_id.as_deref(), Some(run.id.as_str()));

    rt.send_due().await.unwrap();
    assert_eq!(row(&rt, &lost.id).await.status, ScheduledStatus::Sent);
    rt.send_due().await.unwrap();
    assert_eq!(runs_of(&rt, &dot).await, 2);
    rt.shutdown();
}

#[tokio::test]
async fn the_sender_sends_on_the_clock() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let rt = Arc::new(rt);
    let dot = dot(&rt, &dir).await;
    let mut rx = rt.subscribe();
    let sender = rt.spawn_send_later();
    let message = schedule(&rt, &dot, None, TimeDelta::minutes(5)).await;

    // Nothing goes before its time, whatever the loop does meanwhile.
    clock.set(t0() + TimeDelta::minutes(4));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        row(&rt, &message.id).await.status,
        ScheduledStatus::Scheduled
    );

    clock.set(t0() + TimeDelta::minutes(5));
    let sent = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(RuntimeEvent::ScheduledMessageUpdated { message: m }) = rx.recv().await {
                if m.id == message.id && m.status == ScheduledStatus::Sent {
                    return m;
                }
            }
        }
    })
    .await
    .expect("the sender sent the message at its time");
    assert_eq!(sent.sent_at, Some(at(TimeDelta::minutes(5))));
    assert_eq!(runs_of(&rt, &dot).await, 1);
    rt.shutdown();
    sender.await.unwrap();
}
