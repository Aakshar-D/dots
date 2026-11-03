//! The watchers service (watchers spec sections 5 and 7): one queued run per event with the
//! watcher's id and name, the 20-an-hour cap that pauses once, the person's resume, the pauses
//! that stop everything, the acting rule's clock, and the backoff of failures.

mod common;

use chrono::TimeDelta;
use common::clock::FakeClock;
use common::watchers::{after, saturday_10, t0, weekdays_9_to_18, Harness};
use dots_core::model::{
    FolderConfig, PausedReason, RunStatus, TriggerKind, WatchHours, WatcherConfig,
};
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

async fn setup_hours(
    hours: Option<WatchHours>,
) -> (Harness, dots_core::model::Dot, dots_core::model::Watcher) {
    let h = Harness::new().await;
    let dot = h.dot("invoices").await;
    let path = h.folder("in");
    let config = WatcherConfig::Folder(FolderConfig::new(&Harness::path_text(&path)));
    let w = match hours {
        Some(hours) => {
            h.watcher_with_hours(&dot, "Invoices folder", config, hours)
                .await
        }
        None => h.watcher(&dot, "Invoices folder", config).await,
    };
    (h, dot, w)
}

fn with_file(name: &str) -> SourceEvent {
    let mut event = created(name);
    event.files = vec![(name.to_string(), b"%PDF".to_vec())];
    event
}

#[tokio::test]
async fn an_event_outside_hours_is_held_not_run() {
    let (mut h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    h.clock.set(saturday_10());
    h.updated();
    assert!(matches!(
        h.watchers.fire(&w.id, created("a.pdf")).await.unwrap(),
        Fired::Held
    ));
    assert!(h.runs(&dot.id).await.is_empty());
    assert_eq!(h.store.held_count(&w.id).await.unwrap(), 1);
    let held = h.store.held_events(&w.id).await.unwrap();
    assert_eq!(
        held[0].payload,
        json!({"source": "folder", "event": "created", "path": "C:/in/a.pdf",
            "relative": "a.pdf", "size": 3, "watcher_id": w.id, "watcher_name": "Invoices folder"})
    );
    assert_eq!(held[0].files, None);
    assert_eq!(held[0].held_at, timestamp(saturday_10()));
    assert_eq!(h.updated(), vec![w.id.clone()]);
    // Holding does not count against the hour's cap.
    for _ in 0..25 {
        assert!(matches!(
            h.watchers.fire(&w.id, created("b.pdf")).await.unwrap(),
            Fired::Held
        ));
    }
    assert_eq!(
        h.store.get_watcher(&w.id).await.unwrap().paused_reason,
        None
    );
}

#[tokio::test]
async fn an_event_inside_hours_runs_as_today() {
    let (h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    assert!(matches!(
        h.watchers.fire(&w.id, created("a.pdf")).await.unwrap(),
        Fired::Run(_)
    ));
    assert_eq!(h.runs(&dot.id).await.len(), 1);
    assert_eq!(h.store.held_count(&w.id).await.unwrap(), 0);
}

#[tokio::test]
async fn a_watcher_without_hours_runs_any_time() {
    let (h, dot, w) = setup_hours(None).await;
    h.clock.set(saturday_10());
    assert!(matches!(
        h.watchers.fire(&w.id, created("a.pdf")).await.unwrap(),
        Fired::Run(_)
    ));
    assert_eq!(h.runs(&dot.id).await.len(), 1);
    assert_eq!(h.store.held_count(&w.id).await.unwrap(), 0);
}

#[tokio::test]
async fn a_watcher_off_outside_hours_holds_nothing() {
    let (h, _dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    h.clock.set(saturday_10());
    h.watchers.set_enabled(&w.id, false).await.unwrap();
    assert!(matches!(
        h.watchers.fire(&w.id, created("a.pdf")).await.unwrap(),
        Fired::Off
    ));
    h.watchers.set_enabled(&w.id, true).await.unwrap();
    h.watchers.set_paused(true);
    assert!(matches!(
        h.watchers.fire(&w.id, created("a.pdf")).await.unwrap(),
        Fired::Off
    ));
    assert_eq!(h.store.held_count(&w.id).await.unwrap(), 0);
}

#[tokio::test]
async fn a_watcher_at_its_opening_holds_even_inside_its_hours() {
    let (h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    h.watchers.set_opening(&w.id, true);
    assert!(matches!(
        h.watchers.fire(&w.id, created("a.pdf")).await.unwrap(),
        Fired::Held
    ));
    h.watchers.set_opening(&w.id, false);
    assert!(matches!(
        h.watchers.fire(&w.id, created("b.pdf")).await.unwrap(),
        Fired::Run(_)
    ));
    assert_eq!(h.runs(&dot.id).await.len(), 1);
    assert_eq!(h.store.held_count(&w.id).await.unwrap(), 1);
}

#[tokio::test]
async fn the_101st_held_event_stages_no_files() {
    let (h, _dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    h.clock.set(saturday_10());
    for i in 0..101 {
        assert!(matches!(
            h.watchers
                .fire(&w.id, with_file(&format!("{i}.pdf")))
                .await
                .unwrap(),
            Fired::Held
        ));
    }
    let held = h.store.held_events(&w.id).await.unwrap();
    assert_eq!(held.len(), 101);
    let files = h.watchers.files();
    for (i, event) in held.iter().take(100).enumerate() {
        let name = format!("{i}.pdf");
        assert_eq!(event.files, Some(json!([name])));
        let dir = files.held_dir(&w.id, &event.id);
        assert_eq!(std::fs::read(dir.join(&name)).unwrap(), b"%PDF");
    }
    assert_eq!(held[100].payload["relative"], "100.pdf");
    assert_eq!(held[100].files, None);
    assert!(!files.held_dir(&w.id, &held[100].id).exists());
}

#[tokio::test]
async fn hours_are_checked_and_saved_in_week_order() {
    let (h, dot, w) = setup_hours(None).await;
    let config = w.config.clone();
    let mut none = weekdays_9_to_18();
    none.days.clear();
    let refused = h
        .watchers
        .add(&dot.id, "x", config.clone(), Some(none.clone()))
        .await
        .unwrap_err();
    assert!(
        matches!(&refused, dots_core::Error::Invalid(m) if m == "Watch hours need at least one day."),
        "{refused}"
    );
    assert!(h
        .watchers
        .update(&w.id, "x", config.clone(), Some(none))
        .await
        .is_err());
    let mut shuffled = weekdays_9_to_18();
    shuffled.days.reverse();
    let saved = h
        .watchers
        .update(&w.id, "Invoices folder", config.clone(), Some(shuffled))
        .await
        .unwrap();
    assert_eq!(saved.hours, Some(weekdays_9_to_18()));
    let cleared = h
        .watchers
        .update(&w.id, "Invoices folder", config, None)
        .await
        .unwrap();
    assert_eq!(cleared.hours, None);
}
