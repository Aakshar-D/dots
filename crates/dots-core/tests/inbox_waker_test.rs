//! The wake pass and its loop (snooze and done spec section 4.5), on a `FakeClock`: a snooze
//! comes back at its time and not before, with one `SnoozeEnded` per pass; an item that left
//! never comes back; and snoozes that came due while Dots was closed wake at start.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::spec;
use dots_core::events::RuntimeEvent;
use dots_core::inbox::{InboxKey, InboxKind, InboxMark};
use dots_core::model::{Dot, DotSpec, Message, NewRun, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::{Config, Runtime};
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-04T10:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// A runtime on `data` at `now`, with its queue held.
async fn runtime(data: &Path, now: DateTime<Utc>) -> (Arc<Runtime>, Arc<FakeClock>) {
    let clock = FakeClock::new(now);
    let mut cfg = Config::new(data.to_path_buf());
    cfg.port = 0;
    cfg.clock = clock.clone();
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    let rt = Runtime::start(cfg).await.unwrap();
    rt.hold_runs(true).await;
    (Arc::new(rt), clock)
}

async fn dot(rt: &Runtime, dir: &TempDir) -> Dot {
    let mut s: DotSpec = spec("courier");
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    rt.create_dot(s, None, None).await.unwrap()
}

/// An open question of a run that ended.
async fn question(rt: &Runtime, dot: &Dot, text: &str) -> Message {
    let store = rt.store();
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    assert!(store.claim_run(&run.id).await.unwrap());
    store.mark_delivered(&run.id, Utc::now()).await.unwrap();
    store
        .finish_run(&run.id, RunStatus::Succeeded, Some("done"), None)
        .await
        .unwrap();
    store
        .add_question(&dot.id, &run.id, text, None)
        .await
        .unwrap()
        .unwrap()
}

fn key(m: &Message) -> InboxKey {
    InboxKey::new(InboxKind::Message, &m.id)
}

async fn snooze(rt: &Runtime, items: &[InboxKey], until: DateTime<Utc>) {
    rt.inbox_snooze(items, until).await.unwrap();
}

/// The `InboxUpdated` and `SnoozeEnded` events sent so far, in order.
fn inbox_events(rx: &mut Receiver<RuntimeEvent>) -> Vec<RuntimeEvent> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if matches!(
            ev,
            RuntimeEvent::InboxUpdated | RuntimeEvent::SnoozeEnded { .. }
        ) {
            out.push(ev);
        }
    }
    out
}

/// Waits (at most 5 real seconds) for the next event that `pick` takes.
async fn next<T>(rx: &mut Receiver<RuntimeEvent>, pick: impl Fn(RuntimeEvent) -> Option<T>) -> T {
    let wait = async {
        loop {
            if let Some(t) = pick(rx.recv().await.unwrap()) {
                return t;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), wait)
        .await
        .expect("the event came")
}

fn ended(ev: RuntimeEvent) -> Option<Vec<InboxMark>> {
    match ev {
        RuntimeEvent::SnoozeEnded { items } => Some(items),
        _ => None,
    }
}

#[tokio::test]
async fn a_snooze_wakes_at_its_time_and_not_before() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = runtime(&dir.path().join("data"), t0()).await;
    let dot = dot(&rt, &dir).await;
    let q = question(&rt, &dot, "Ship it?").await;
    snooze(&rt, &[key(&q)], t0() + TimeDelta::hours(1)).await;
    assert_eq!(rt.inbox_count().await.unwrap(), 0);
    let mut events = rt.subscribe();

    clock.set(t0() + TimeDelta::minutes(59));
    assert!(rt.wake_snoozed().await.unwrap().is_empty());
    assert_eq!(rt.inbox().await.unwrap().snoozed.len(), 1);
    assert!(inbox_events(&mut events).is_empty());

    clock.set(t0() + TimeDelta::hours(1));
    let woke = rt.wake_snoozed().await.unwrap();
    assert_eq!(woke.len(), 1);
    assert_eq!(woke[0].key(), key(&q));
    assert_eq!(woke[0].title, "courier asks: Ship it?");
    assert_eq!(woke[0].dot_id.as_deref(), Some(dot.id.as_str()));
    let inbox = rt.inbox().await.unwrap();
    assert!(inbox.snoozed.is_empty());
    assert_eq!(inbox.back.len(), 1);
    assert_eq!(inbox.back[0].key(), key(&q));
    assert!(inbox.hidden_messages.is_empty());
    assert_eq!(rt.inbox_count().await.unwrap(), 1);
    assert_eq!(rt.store().dot_counts(&dot.id).await.unwrap().0, 1);

    let sent = inbox_events(&mut events);
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(matches!(sent[0], RuntimeEvent::InboxUpdated));
    match &sent[1] {
        RuntimeEvent::SnoozeEnded { items } => assert_eq!(items, &woke),
        other => panic!("expected SnoozeEnded, got {other:?}"),
    }
    rt.shutdown();
}

#[tokio::test]
async fn several_wake_in_one_event() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = runtime(&dir.path().join("data"), t0()).await;
    let dot = dot(&rt, &dir).await;
    let mut keys = Vec::new();
    for text in ["One?", "Two?", "Three?"] {
        keys.push(key(&question(&rt, &dot, text).await));
    }
    snooze(&rt, &keys, t0() + TimeDelta::minutes(1)).await;
    let mut events = rt.subscribe();
    clock.set(t0() + TimeDelta::minutes(1));
    assert_eq!(rt.wake_snoozed().await.unwrap().len(), 3);
    let ends: Vec<Vec<InboxMark>> = inbox_events(&mut events)
        .into_iter()
        .filter_map(ended)
        .collect();
    assert_eq!(ends.len(), 1);
    assert_eq!(ends[0].len(), 3);
    rt.shutdown();
}

#[tokio::test]
async fn a_snoozed_item_that_left_never_comes_back() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = runtime(&dir.path().join("data"), t0()).await;
    let dot = dot(&rt, &dir).await;
    let q = question(&rt, &dot, "Ship it?").await;
    snooze(&rt, &[key(&q)], t0() + TimeDelta::hours(1)).await;
    rt.reply(&q.id, "yes").await.unwrap();
    // It left: not listed while its mark waits for the pass.
    assert!(rt.inbox().await.unwrap().snoozed.is_empty());

    let mut events = rt.subscribe();
    clock.set(t0() + TimeDelta::hours(2));
    assert!(rt.wake_snoozed().await.unwrap().is_empty());
    let sent = inbox_events(&mut events);
    assert!(
        !sent
            .iter()
            .any(|e| matches!(e, RuntimeEvent::SnoozeEnded { .. })),
        "{sent:?}"
    );
    assert_eq!(rt.store().count_marks().await.unwrap(), 0);
    assert!(rt.inbox().await.unwrap().back.is_empty());
    rt.shutdown();
}

#[tokio::test]
async fn snoozes_due_while_closed_wake_at_start() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let q = {
        let (rt, _clock) = runtime(&data, t0()).await;
        let dot = dot(&rt, &dir).await;
        let q = question(&rt, &dot, "Ship it?").await;
        snooze(&rt, &[key(&q)], t0() + TimeDelta::hours(1)).await;
        assert!(rt.close(Duration::from_secs(10)).await);
        q
    };

    let (rt, _clock) = runtime(&data, t0() + TimeDelta::hours(3)).await;
    let mut events = rt.subscribe();
    let _waker = rt.spawn_snooze_waker();
    let items = next(&mut events, ended).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].key(), key(&q));
    assert_eq!(items[0].title, "courier asks: Ship it?");

    // Nothing more to wake: the next pass sends no second notice.
    assert!(rt.wake_snoozed().await.unwrap().is_empty());
    let more: Vec<_> = inbox_events(&mut events)
        .into_iter()
        .filter_map(ended)
        .collect();
    assert!(more.is_empty());
    assert_eq!(rt.inbox().await.unwrap().back.len(), 1);
    rt.shutdown();
}

#[tokio::test]
async fn the_waker_runs_on_the_clock() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = runtime(&dir.path().join("data"), t0()).await;
    let dot = dot(&rt, &dir).await;
    let gone = question(&rt, &dot, "Gone?").await;
    let later = question(&rt, &dot, "Later?").await;
    snooze(&rt, &[key(&gone)], t0() + TimeDelta::hours(1)).await;
    rt.reply(&gone.id, "yes").await.unwrap();
    snooze(&rt, &[key(&later)], t0() + TimeDelta::seconds(10)).await;

    let mut events = rt.subscribe();
    let _waker = rt.spawn_snooze_waker();
    // The first pass, at once: it deletes the mark of the item that left, and wakes nothing.
    next(&mut events, |ev| {
        matches!(ev, RuntimeEvent::InboxUpdated).then_some(())
    })
    .await;
    assert_eq!(rt.inbox().await.unwrap().snoozed.len(), 1);

    // Its next pass is 30 seconds of the clock after the first.
    clock.set(t0() + TimeDelta::seconds(30));
    let items = next(&mut events, ended).await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].key(), key(&later));
    rt.shutdown();
}

#[tokio::test]
async fn no_marks_no_work() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = runtime(&dir.path().join("data"), t0()).await;
    let dot = dot(&rt, &dir).await;
    question(&rt, &dot, "Ship it?").await;
    let mut events = rt.subscribe();
    clock.set(t0() + TimeDelta::hours(1));
    assert!(rt.wake_snoozed().await.unwrap().is_empty());
    assert!(inbox_events(&mut events).is_empty());
    rt.shutdown();
}
