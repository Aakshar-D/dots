//! The watchers service (watchers spec sections 5 and 7): one queued run per event with the
//! watcher's id and name, the 20-an-hour cap that pauses once, the person's resume, the pauses
//! that stop everything, the acting rule's clock, and the backoff of failures.

mod common;

use chrono::TimeDelta;
use common::clock::FakeClock;
use common::watchers::{after, monday_9, saturday_10, t0, weekdays_9_to_18, Harness};
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

/// Every day, 00:00 to 23:59: inside at any moment but the last minute of a day.
fn all_day() -> WatchHours {
    use chrono::{NaiveTime, Weekday};
    WatchHours {
        days: vec![
            Weekday::Mon,
            Weekday::Tue,
            Weekday::Wed,
            Weekday::Thu,
            Weekday::Fri,
            Weekday::Sat,
            Weekday::Sun,
        ],
        start: NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
        end: NaiveTime::from_hms_opt(23, 59, 0).unwrap(),
    }
}

/// Holds `n` events on Saturday, one second apart, oldest first.
async fn hold(h: &Harness, w: &dots_core::model::Watcher, n: usize, files: bool) {
    for i in 0..n {
        h.clock.set(saturday_10() + TimeDelta::seconds(i as i64));
        let name = format!("{i}.pdf");
        let event = if files {
            with_file(&name)
        } else {
            created(&name)
        };
        assert!(matches!(
            h.watchers.fire(&w.id, event).await.unwrap(),
            Fired::Held
        ));
    }
}

fn held_names(run: &dots_core::model::Run) -> Vec<String> {
    run.payload.as_ref().unwrap()["held"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["relative"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn the_opening_runs_once_with_everything_held() {
    let (h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 3, false).await;
    h.start();
    // The folder watcher's opening timer sleeps until Monday 09:00.
    assert_eq!(h.clock.nth_sleep(1).await, monday_9());
    assert!(h.runs(&dot.id).await.is_empty());
    h.clock.set(monday_9());
    let runs = h.wait_runs(&dot.id, 1, 10).await;
    let p = runs[0].payload.as_ref().unwrap();
    assert_eq!(p["watcher_id"], json!(w.id));
    assert_eq!(p["watcher_name"], "Invoices folder");
    assert_eq!(held_names(&runs[0]), ["0.pdf", "1.pdf", "2.pdf"]);
    assert_eq!(p["held"][0]["watcher_id"], json!(w.id));
    assert_eq!(p["held_more"], 0);
    assert_eq!(p["held_since"], json!(timestamp(saturday_10())));
    assert_eq!(p["attachments"], json!([]));
    // The timer waits for the next opening once the opening is done (the run is made before
    // the held events are taken); nothing more runs.
    assert_eq!(h.clock.nth_sleep(2).await, monday_9() + TimeDelta::days(1));
    assert_eq!(h.store.held_count(&w.id).await.unwrap(), 0);
    h.still(&dot.id, 1, 1).await;
}

#[tokio::test]
async fn the_opening_carries_100_and_counts_the_rest() {
    let (h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 130, false).await;
    h.clock.set(monday_9());
    let Fired::Run(run) = h.watchers.open(&w.id).await.unwrap() else {
        panic!("a run")
    };
    let names = held_names(&run);
    assert_eq!(names.len(), 100);
    assert_eq!(names[0], "0.pdf");
    assert_eq!(names[99], "99.pdf");
    assert_eq!(run.payload.as_ref().unwrap()["held_more"], 30);
    assert_eq!(h.runs(&dot.id).await.len(), 1);
    assert_eq!(h.store.held_count(&w.id).await.unwrap(), 0);
    // Nothing is held: a second opening does nothing.
    assert!(matches!(h.watchers.open(&w.id).await.unwrap(), Fired::Off));
}

#[tokio::test]
async fn the_opening_counts_once_against_the_flood_cap() {
    let (h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 30, false).await;
    h.clock.set(monday_9());
    assert!(matches!(
        h.watchers.open(&w.id).await.unwrap(),
        Fired::Run(_)
    ));
    for _ in 0..19 {
        assert!(matches!(
            h.watchers.fire(&w.id, created("a.pdf")).await.unwrap(),
            Fired::Run(_)
        ));
    }
    assert!(matches!(
        h.watchers.fire(&w.id, created("b.pdf")).await.unwrap(),
        Fired::Capped
    ));
    assert_eq!(h.runs(&dot.id).await.len(), 20);
}

#[tokio::test]
async fn an_off_or_capped_opening_keeps_the_events() {
    let (h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 2, true).await;
    h.clock.set(monday_9());
    h.watchers.set_paused(true);
    assert!(matches!(h.watchers.open(&w.id).await.unwrap(), Fired::Off));
    assert_eq!(h.store.held_count(&w.id).await.unwrap(), 2);
    h.watchers.set_paused(false);
    for _ in 0..20 {
        assert!(matches!(
            h.watchers.fire(&w.id, created("a.pdf")).await.unwrap(),
            Fired::Run(_)
        ));
    }
    assert!(matches!(
        h.watchers.open(&w.id).await.unwrap(),
        Fired::Capped
    ));
    assert_eq!(h.runs(&dot.id).await.len(), 20);
    let paused = h.store.get_watcher(&w.id).await.unwrap();
    assert_eq!(paused.paused_reason, Some(PausedReason::Flood));
    assert_eq!(paused.held_count, 2);
    // Their files are kept too.
    for event in h.store.held_events(&w.id).await.unwrap() {
        assert!(h.watchers.files().held_dir(&w.id, &event.id).is_dir());
    }
}

/// Review Focus 2: Dots closed at the opening and started after it releases once, not once
/// at start and again from the timer.
#[tokio::test]
async fn held_events_are_released_at_start_once() {
    let (mut h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 2, false).await;
    h.clock.set(monday_9() + TimeDelta::minutes(30));
    h.restart();
    h.start();
    let runs = h.wait_runs(&dot.id, 1, 10).await;
    assert_eq!(held_names(&runs[0]), ["0.pdf", "1.pdf"]);
    // The opening timer waits for Tuesday's opening, which finds nothing held.
    assert_eq!(h.clock.nth_sleep(1).await, monday_9() + TimeDelta::days(1));
    h.clock.set(monday_9() + TimeDelta::days(1));
    assert_eq!(h.clock.nth_sleep(2).await, monday_9() + TimeDelta::days(2));
    h.still(&dot.id, 1, 1).await;
}

/// Review Focus 5: hours edited from 09–18 to 00–23:59 while holding release at once.
#[tokio::test]
async fn editing_hours_so_now_is_inside_releases_at_once() {
    let (h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 2, false).await;
    // Edited but still outside: kept.
    let mut later = weekdays_9_to_18();
    later.start = chrono::NaiveTime::from_hms_opt(11, 0, 0).unwrap();
    let saved = h
        .watchers
        .update(&w.id, "Invoices folder", w.config.clone(), Some(later))
        .await
        .unwrap();
    assert_eq!(saved.held_count, 2);
    assert!(h.runs(&dot.id).await.is_empty());
    let saved = h
        .watchers
        .update(&w.id, "Invoices folder", w.config.clone(), Some(all_day()))
        .await
        .unwrap();
    let runs = h.runs(&dot.id).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(held_names(&runs[0]), ["0.pdf", "1.pdf"]);
    assert_eq!(saved.held_count, 0);
    assert_eq!(saved.next_opening, None);
}

#[tokio::test]
async fn removing_hours_releases_at_once() {
    let (h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 2, false).await;
    let saved = h
        .watchers
        .update(&w.id, "Invoices folder", w.config.clone(), None)
        .await
        .unwrap();
    assert_eq!(h.runs(&dot.id).await.len(), 1);
    assert_eq!(saved.held_count, 0);
}

#[tokio::test]
async fn resuming_inside_hours_releases() {
    let (h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 2, false).await;
    // Resumed outside its hours: still held.
    h.watchers
        .pause(&w.id, PausedReason::Flood, None)
        .await
        .unwrap();
    let resumed = h.watchers.resume(&w.id).await.unwrap();
    assert_eq!(resumed.held_count, 2);
    assert!(h.runs(&dot.id).await.is_empty());
    h.clock.set(monday_9() + TimeDelta::hours(1));
    h.watchers
        .pause(&w.id, PausedReason::Flood, None)
        .await
        .unwrap();
    let resumed = h.watchers.resume(&w.id).await.unwrap();
    assert_eq!(resumed.held_count, 0);
    let runs = h.runs(&dot.id).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(held_names(&runs[0]), ["0.pdf", "1.pdf"]);
}

#[tokio::test]
async fn turning_off_drops_held_events_and_their_files() {
    let (h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 2, true).await;
    let held = h.store.held_events(&w.id).await.unwrap();
    let off = h.watchers.set_enabled(&w.id, false).await.unwrap();
    assert_eq!(off.held_count, 0);
    assert_eq!(h.store.held_count(&w.id).await.unwrap(), 0);
    for event in &held {
        assert!(!h.watchers.files().held_dir(&w.id, &event.id).exists());
    }
    h.clock.set(monday_9());
    h.watchers.set_enabled(&w.id, true).await.unwrap();
    assert!(h.runs(&dot.id).await.is_empty());
}

#[tokio::test]
async fn deleting_drops() {
    let (h, _dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 2, true).await;
    let held = h.store.held_events(&w.id).await.unwrap();
    h.watchers.delete(&w.id).await.unwrap();
    assert_eq!(h.store.held_count(&w.id).await.unwrap(), 0);
    for event in &held {
        assert!(!h.watchers.files().held_dir(&w.id, &event.id).exists());
    }
}

#[tokio::test]
async fn next_opening_is_set_outside_the_hours_and_none_inside() {
    let (h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    assert_eq!(
        h.watchers.list(&dot.id).await.unwrap()[0].next_opening,
        None
    );
    h.clock.set(saturday_10());
    let listed = &h.watchers.list(&dot.id).await.unwrap()[0];
    assert_eq!(
        listed.next_opening.as_deref(),
        Some("2026-10-26T09:00:00+01:00")
    );
    let resumed = h.watchers.resume(&w.id).await.unwrap();
    assert_eq!(
        resumed.next_opening.as_deref(),
        Some("2026-10-26T09:00:00+01:00")
    );
    let off = h.watchers.set_enabled(&w.id, false).await.unwrap();
    assert!(off.next_opening.is_some());
    let none = h
        .watchers
        .update(&w.id, "Invoices folder", w.config.clone(), None)
        .await
        .unwrap();
    assert_eq!(none.next_opening, None);
}

/// Carried over: a held row's files are best-effort; one that is gone is left out of the
/// opening, which still runs with every event.
#[tokio::test]
async fn a_held_file_that_is_gone_is_left_out_of_the_opening() {
    let (h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 2, true).await;
    let held = h.store.held_events(&w.id).await.unwrap();
    let files = h.watchers.files();
    std::fs::remove_file(files.held_dir(&w.id, &held[0].id).join("0.pdf")).unwrap();
    h.clock.set(monday_9());
    let Fired::Run(run) = h.watchers.open(&w.id).await.unwrap() else {
        panic!("a run")
    };
    assert_eq!(held_names(&run), ["0.pdf", "1.pdf"]);
    assert_eq!(
        run.payload.as_ref().unwrap()["attachments"],
        json!([{"name": "2-1.pdf", "size": 4, "path": null, "event": 2}])
    );
    let staged = files.stage_dir(&w.id, &run.id);
    assert_eq!(std::fs::read(staged.join("2-1.pdf")).unwrap(), b"%PDF");
    for event in &held {
        assert!(!files.held_dir(&w.id, &event.id).exists());
    }
    assert_eq!(h.runs(&dot.id).await.len(), 1);
}

/// Carried over from Task 1: hours that fail their check (only a damaged row can hold them)
/// count as any time; the service neither holds forever nor spins on an opening in the past.
#[tokio::test]
async fn hours_that_fail_their_check_count_as_any_time() {
    let (h, dot, w) = setup_hours(None).await;
    let mut broken = weekdays_9_to_18();
    broken.days.clear();
    h.store
        .update_watcher(&w.id, &w.name, &w.config, Some(&broken))
        .await
        .unwrap();
    h.clock.set(saturday_10());
    assert!(matches!(
        h.watchers.fire(&w.id, created("a.pdf")).await.unwrap(),
        Fired::Run(_)
    ));
    assert_eq!(
        h.watchers.list(&dot.id).await.unwrap()[0].next_opening,
        None
    );
    h.start();
    h.watching(&w.id).await;
    h.still(&dot.id, 1, 1).await;
    assert_eq!(h.clock.sleeps(), 0);
}

/// Monday 26 October 2026, 19:00 in `TestTz`: after the day's window, outside again.
fn monday_19() -> chrono::DateTime<chrono::Utc> {
    monday_9() + TimeDelta::hours(10)
}

/// Final review I1 (spec section 3.8): Dots closed over the whole opening and started outside
/// the hours again releases what it held, once; the timer then waits for the next opening.
#[tokio::test]
async fn held_events_are_released_at_start_after_a_missed_opening() {
    let (mut h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 2, true).await;
    h.clock.set(monday_19());
    h.restart();
    h.start();
    let runs = h.wait_runs(&dot.id, 1, 10).await;
    assert_eq!(held_names(&runs[0]), ["0.pdf", "1.pdf"]);
    assert_eq!(
        runs[0].payload.as_ref().unwrap()["held_since"],
        json!(timestamp(saturday_10()))
    );
    assert_eq!(h.store.held_count(&w.id).await.unwrap(), 0);
    assert_eq!(h.clock.nth_sleep(1).await, monday_9() + TimeDelta::days(1));
    h.still(&dot.id, 1, 1).await;
}

/// Final review I1: Dots paused across a whole window and unpaused outside the hours releases.
#[tokio::test]
async fn held_events_are_released_when_dots_is_unpaused_after_a_missed_opening() {
    let (h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 2, false).await;
    h.watchers.set_paused(true);
    h.start();
    h.clock.set(monday_19());
    h.still(&dot.id, 0, 1).await;
    h.watchers.set_paused(false);
    let runs = h.wait_runs(&dot.id, 1, 10).await;
    assert_eq!(held_names(&runs[0]), ["0.pdf", "1.pdf"]);
    assert_eq!(h.store.held_count(&w.id).await.unwrap(), 0);
    h.still(&dot.id, 1, 1).await;
}

/// Final review I1: a resume outside the hours releases when an opening has passed since the
/// oldest held event; with no opening passed it keeps them (`resuming_inside_hours_releases`).
#[tokio::test]
async fn resuming_after_a_missed_opening_releases() {
    let (h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 2, false).await;
    h.watchers
        .pause(&w.id, PausedReason::Flood, None)
        .await
        .unwrap();
    h.clock.set(monday_19());
    let resumed = h.watchers.resume(&w.id).await.unwrap();
    assert_eq!(resumed.held_count, 0);
    let runs = h.runs(&dot.id).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(held_names(&runs[0]), ["0.pdf", "1.pdf"]);
}

/// Final review I2: the run and the taking of the held events are one step. A failure of it
/// (here the delete of the held rows refused) leaves no run, the events held and their files
/// back in their held folders; once it works, one run carries them and nothing stays held.
#[tokio::test]
async fn an_opening_that_cannot_take_the_held_events_makes_no_run_and_keeps_them() {
    let (h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 2, true).await;
    let held = h.store.held_events(&w.id).await.unwrap();
    h.exec(
        "CREATE TRIGGER keep_held BEFORE DELETE ON watcher_held \
         BEGIN SELECT RAISE(ABORT, 'refused'); END",
    )
    .await;
    h.clock.set(monday_9());
    assert!(h.watchers.open(&w.id).await.is_err());
    assert!(h.runs(&dot.id).await.is_empty());
    assert_eq!(h.store.held_events(&w.id).await.unwrap(), held);
    let files = h.watchers.files();
    for (i, event) in held.iter().enumerate() {
        let file = files.held_dir(&w.id, &event.id).join(format!("{i}.pdf"));
        assert_eq!(std::fs::read(file).unwrap(), b"%PDF");
    }
    h.exec("DROP TRIGGER keep_held").await;
    let Fired::Run(run) = h.watchers.open(&w.id).await.unwrap() else {
        panic!("a run")
    };
    assert_eq!(held_names(&run), ["0.pdf", "1.pdf"]);
    assert_eq!(h.runs(&dot.id).await.len(), 1);
    assert_eq!(h.store.held_count(&w.id).await.unwrap(), 0);
    for event in &held {
        assert!(!files.held_dir(&w.id, &event.id).exists());
    }
    let staged = files.stage_dir(&w.id, &run.id);
    assert_eq!(std::fs::read(staged.join("1-0.pdf")).unwrap(), b"%PDF");
    assert_eq!(std::fs::read(staged.join("2-1.pdf")).unwrap(), b"%PDF");
}

/// Final review I3: an edit whose release fails is still saved, announced and answered; the
/// events stay held for the next try.
#[tokio::test]
async fn an_edit_whose_release_fails_is_still_announced() {
    let (mut h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 2, false).await;
    h.updated();
    h.exec(
        "CREATE TRIGGER no_runs BEFORE INSERT ON runs BEGIN SELECT RAISE(FAIL, 'disk full'); END",
    )
    .await;
    let saved = h
        .watchers
        .update(&w.id, "Renamed", w.config.clone(), Some(all_day()))
        .await
        .unwrap();
    assert_eq!(saved.name, "Renamed");
    assert_eq!(saved.held_count, 2);
    assert!(h.updated().contains(&w.id));
    assert!(h.runs(&dot.id).await.is_empty());
    // Turned on (off behind the service's back, so the events stay) and resumed: the same.
    h.store.set_watcher_enabled(&w.id, false).await.unwrap();
    h.updated();
    let on = h.watchers.set_enabled(&w.id, true).await.unwrap();
    assert!(on.enabled);
    assert_eq!(on.held_count, 2);
    assert!(h.updated().contains(&w.id));
    h.watchers
        .pause(&w.id, PausedReason::Flood, None)
        .await
        .unwrap();
    h.updated();
    let resumed = h.watchers.resume(&w.id).await.unwrap();
    assert_eq!(resumed.paused_reason, None);
    assert_eq!(resumed.held_count, 2);
    assert!(h.updated().contains(&w.id));
    assert!(h.runs(&dot.id).await.is_empty());
}

/// Final review M2: a folder watcher's opening that fails is tried again after the backoff's
/// first wait (1 minute), not at the next day's opening.
#[tokio::test]
async fn a_failed_folder_opening_is_tried_again_after_a_minute() {
    let (h, dot, w) = setup_hours(Some(weekdays_9_to_18())).await;
    hold(&h, &w, 2, false).await;
    h.start();
    assert_eq!(h.clock.nth_sleep(1).await, monday_9());
    h.exec(
        "CREATE TRIGGER no_runs BEFORE INSERT ON runs BEGIN SELECT RAISE(FAIL, 'disk full'); END",
    )
    .await;
    h.clock.set(monday_9());
    assert_eq!(
        h.clock.nth_sleep(2).await,
        monday_9() + TimeDelta::minutes(1)
    );
    assert!(h.runs(&dot.id).await.is_empty());
    assert_eq!(h.store.held_count(&w.id).await.unwrap(), 2);
    h.exec("DROP TRIGGER no_runs").await;
    h.clock.set(monday_9() + TimeDelta::minutes(1));
    let runs = h.wait_runs(&dot.id, 1, 10).await;
    assert_eq!(held_names(&runs[0]), ["0.pdf", "1.pdf"]);
    assert_eq!(h.clock.nth_sleep(3).await, monday_9() + TimeDelta::days(1));
    h.still(&dot.id, 1, 1).await;
}
