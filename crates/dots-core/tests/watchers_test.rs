//! The watchers service (watchers spec sections 5 and 7): one queued run per event with the
//! watcher's id and name, the 20-an-hour cap that pauses once, the person's resume, the pauses
//! that stop everything, the acting rule's clock, and the backoff of failures.

mod common;

use chrono::TimeDelta;
use common::clock::FakeClock;
use common::watchers::{after, t0, Harness};
use dots_core::model::{FolderConfig, PausedReason, RunStatus, TriggerKind, WatcherConfig};
use dots_core::util::timestamp;
use dots_core::watchers::{ActingTracker, Backoff, Failure, Fired, Poll, SourceEvent};
use serde_json::json;

fn created(name: &str) -> SourceEvent {
    SourceEvent::new(json!({"source": "folder", "event": "created",
        "path": format!("C:/in/{name}"), "relative": name, "size": 3}))
}

async fn setup() -> (Harness, dots_core::model::Dot, dots_core::model::Watcher) {
    let h = Harness::new().await;
    let dot = h.dot("invoices").await;
    let path = h.folder("in");
    let w = h
        .watcher(
            &dot,
            "Invoices folder",
            WatcherConfig::Folder(FolderConfig::new(&Harness::path_text(&path))),
        )
        .await;
    (h, dot, w)
}

#[tokio::test]
async fn an_event_queues_one_watcher_run_carrying_the_watchers_id_and_name() {
    let (h, dot, w) = setup().await;
    let Fired::Run(run) = h.watchers.fire(&w.id, created("a.pdf")).await.unwrap() else {
        panic!("a run")
    };
    assert_eq!(run.dot_id, dot.id);
    assert_eq!(run.trigger, TriggerKind::Watcher);
    assert_eq!(run.status, RunStatus::Queued);
    assert_eq!(
        run.payload,
        Some(
            json!({"source": "folder", "event": "created", "path": "C:/in/a.pdf",
            "relative": "a.pdf", "size": 3, "watcher_id": w.id, "watcher_name": "Invoices folder"})
        )
    );
    assert_eq!(h.runs(&dot.id).await.len(), 1);
    assert_eq!(
        h.store.get_watcher(&w.id).await.unwrap().last_event_at,
        Some(timestamp(t0()))
    );
}

#[tokio::test]
async fn the_21st_event_in_an_hour_pauses_the_watcher_once_and_a_resume_allows_20_more() {
    let (mut h, dot, w) = setup().await;
    for i in 0..20 {
        h.clock.set(after(i * 60));
        assert!(matches!(
            h.watchers.fire(&w.id, created("a.pdf")).await.unwrap(),
            Fired::Run(_)
        ));
    }
    assert!(matches!(
        h.watchers.fire(&w.id, created("b.pdf")).await.unwrap(),
        Fired::Capped
    ));
    let paused = h.store.get_watcher(&w.id).await.unwrap();
    assert_eq!(paused.paused_reason, Some(PausedReason::Flood));
    assert_eq!(paused.last_error, None);
    assert_eq!(h.paused(), vec![(w.id.clone(), None)]);
    // Paused: further events are dropped, and the pause is not announced again.
    assert!(matches!(
        h.watchers.fire(&w.id, created("c.pdf")).await.unwrap(),
        Fired::Off
    ));
    assert!(h.paused().is_empty());
    assert_eq!(h.runs(&dot.id).await.len(), 20);
    assert_eq!(h.store.paused_watchers().await.unwrap().len(), 1);

    let resumed = h.watchers.resume(&w.id).await.unwrap();
    assert_eq!(resumed.paused_reason, None);
    for _ in 0..20 {
        assert!(matches!(
            h.watchers.fire(&w.id, created("d.pdf")).await.unwrap(),
            Fired::Run(_)
        ));
    }
    assert!(matches!(
        h.watchers.fire(&w.id, created("e.pdf")).await.unwrap(),
        Fired::Capped
    ));
    assert_eq!(h.runs(&dot.id).await.len(), 40);
}

#[tokio::test]
async fn an_event_an_hour_after_the_first_counts_from_then() {
    let (h, _dot, w) = setup().await;
    for _ in 0..20 {
        h.watchers.fire(&w.id, created("a.pdf")).await.unwrap();
    }
    h.clock.set(after(3601));
    assert!(matches!(
        h.watchers.fire(&w.id, created("b.pdf")).await.unwrap(),
        Fired::Run(_)
    ));
}

#[tokio::test]
async fn a_watcher_off_paused_or_of_a_disabled_dot_and_dots_paused_start_nothing() {
    let (h, dot, w) = setup().await;
    h.watchers.set_enabled(&w.id, false).await.unwrap();
    assert!(matches!(
        h.watchers.fire(&w.id, created("a.pdf")).await.unwrap(),
        Fired::Off
    ));
    h.watchers.set_enabled(&w.id, true).await.unwrap();

    h.watchers.set_paused(true);
    assert!(h.watchers.is_paused());
    assert!(matches!(
        h.watchers.fire(&w.id, created("a.pdf")).await.unwrap(),
        Fired::Off
    ));
    h.watchers.set_paused(false);

    h.store.set_dot_enabled(&dot.id, false).await.unwrap();
    assert!(matches!(
        h.watchers.fire(&w.id, created("a.pdf")).await.unwrap(),
        Fired::Off
    ));
    h.store.set_dot_enabled(&dot.id, true).await.unwrap();

    h.watchers
        .pause(&w.id, PausedReason::Error, Some("broken"))
        .await
        .unwrap();
    assert!(matches!(
        h.watchers.fire(&w.id, created("a.pdf")).await.unwrap(),
        Fired::Off
    ));
    assert!(h.runs(&dot.id).await.is_empty());
    h.watchers.resume(&w.id).await.unwrap();
    assert!(matches!(
        h.watchers.fire(&w.id, created("a.pdf")).await.unwrap(),
        Fired::Run(_)
    ));
}

#[test]
fn a_dot_acts_while_a_run_is_held_and_five_seconds_after_the_last_ends() {
    let clock = FakeClock::new(t0());
    let acting = ActingTracker::new(clock.clone());
    assert!(!acting.is_acting("d1", t0()));
    acting.begin("d1");
    acting.begin("d1");
    assert!(acting.is_acting("d1", t0()));
    assert!(!acting.is_acting("d2", t0()));
    clock.set(after(10));
    acting.end("d1");
    // One of two ended: still acting, long after.
    assert!(acting.is_acting("d1", after(100)));
    clock.set(after(20));
    acting.end("d1");
    assert!(acting.is_acting("d1", after(20)));
    assert!(acting.is_acting("d1", after(24) + TimeDelta::milliseconds(999)));
    assert!(!acting.is_acting("d1", after(25)));
}

#[test]
fn failures_back_off_1_2_5_10_then_every_10_minutes_and_give_up_after_an_hour() {
    let m = |n: i64| t0() + TimeDelta::minutes(n);
    let mut b = Backoff::new();
    for (now, next) in [
        (0, 1),
        (1, 3),
        (3, 8),
        (8, 18),
        (18, 28),
        (28, 38),
        (38, 48),
        (48, 58),
        (58, 68),
    ] {
        assert_eq!(
            b.failed(m(now)),
            Some(m(next)),
            "after the failure at minute {now}"
        );
    }
    assert_eq!(b.failed(m(68)), None);
    b.succeeded();
    assert_eq!(b.failed(m(100)), Some(m(101)));
}

#[tokio::test]
async fn a_permanent_failure_pauses_at_once_and_a_transient_one_after_an_hour_of_them() {
    let (mut h, _dot, w) = setup().await;
    let next = h
        .watchers
        .note_failure(&w.id, Failure::Transient("network down".into()))
        .await
        .unwrap();
    assert_eq!(next, Poll::Next(t0() + TimeDelta::minutes(1)));
    let now = h.store.get_watcher(&w.id).await.unwrap();
    assert_eq!(
        (now.paused_reason, now.last_error.as_deref()),
        (None, Some("network down"))
    );
    h.watchers.note_success(&w.id).await.unwrap();
    assert_eq!(h.store.get_watcher(&w.id).await.unwrap().last_error, None);

    h.watchers
        .note_failure(&w.id, Failure::Transient("down again".into()))
        .await
        .unwrap();
    h.clock.set(t0() + TimeDelta::minutes(61));
    let stop = h
        .watchers
        .note_failure(&w.id, Failure::Transient("still down".into()))
        .await
        .unwrap();
    assert_eq!(stop, Poll::Stopped);
    let paused = h.store.get_watcher(&w.id).await.unwrap();
    assert_eq!(paused.paused_reason, Some(PausedReason::Error));
    assert_eq!(paused.last_error.as_deref(), Some("still down"));
    assert_eq!(h.paused(), vec![(w.id.clone(), Some("still down".into()))]);

    h.watchers.resume(&w.id).await.unwrap();
    let refused = "Gmail refused the app password for me@gmail.com, or IMAP is off for it.";
    assert_eq!(
        h.watchers
            .note_failure(&w.id, Failure::Permanent(refused.into()))
            .await
            .unwrap(),
        Poll::Stopped
    );
    assert_eq!(h.paused(), vec![(w.id.clone(), Some(refused.into()))]);
}
