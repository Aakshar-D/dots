//! The limits service (usage and limits spec sections 3 and 4): which queued run may start, given
//! the latest readings of the Claude windows, the person's shares, Claude's `rejected` and the
//! one-at-a-time draining after a hold; and the hold's start and end, published once each.

mod common;

use std::sync::Arc;

use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::temp_store;
use dots_core::events::{new_bus, RuntimeEvent};
use dots_core::limits::{Admit, HoldChange, HoldShares, LimitReading, Limits, QueuedRun};
use dots_core::model::{EngineKind, TriggerKind};
use dots_core::scheduler::Clock;
use dots_core::store::Store;
use dots_core::util::timestamp;
use tempfile::TempDir;

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-26T12:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc)
}

const SHARES: HoldShares = HoldShares {
    enabled: true,
    five_hour: 60,
    weekly: 70,
};

fn reading(window: &str, utilization: f64, resets_in: TimeDelta) -> LimitReading {
    LimitReading {
        window: window.into(),
        utilization,
        resets_at: timestamp(t0() + resets_in),
        status: "allowed".into(),
        read_at: timestamp(t0()),
    }
}

fn five(u: f64) -> LimitReading {
    reading("five_hour", u, TimeDelta::hours(3))
}

fn week(u: f64) -> LimitReading {
    reading("seven_day", u, TimeDelta::days(3))
}

fn q(id: &str, trigger: TriggerKind) -> QueuedRun {
    QueuedRun {
        id: id.into(),
        trigger,
        engine: EngineKind::Claude,
        hold_released: false,
        queued_at: timestamp(t0()),
    }
}

struct Env {
    _dir: TempDir,
    store: Store,
    clock: Arc<FakeClock>,
    bus: dots_core::events::Bus,
    limits: Arc<Limits>,
}

async fn env(shares: HoldShares) -> Env {
    let (dir, store) = temp_store().await;
    let clock = FakeClock::new(t0());
    let bus = new_bus();
    let limits = Limits::load(store.clone(), bus.clone(), clock.clone(), shares)
        .await
        .unwrap();
    Env {
        _dir: dir,
        store,
        clock,
        bus,
        limits,
    }
}

#[tokio::test]
async fn local_runs_are_never_held() {
    let e = env(SHARES).await;
    e.limits.record("rejected", vec![five(1.0)]).await.unwrap();
    let mut run = q("r", TriggerKind::Schedule);
    run.engine = EngineKind::Local;
    assert_eq!(e.limits.admit(&run, 3), Admit::Go);
}

#[tokio::test]
async fn the_persons_runs_are_never_held() {
    let e = env(SHARES).await;
    e.limits
        .record("allowed", vec![five(0.95), week(0.95)])
        .await
        .unwrap();
    for t in [TriggerKind::Chat, TriggerKind::Manual, TriggerKind::Resume] {
        assert_eq!(e.limits.admit(&q("r", t), 1), Admit::Go, "{t:?}");
    }
}

#[tokio::test]
async fn background_runs_are_held_at_the_share() {
    let e = env(SHARES).await;
    let sched = q("r", TriggerKind::Schedule);
    for (readings, want) in [
        (vec![five(0.60)], Admit::Held),
        (vec![five(0.59)], Admit::Go),
        (vec![five(0.1), week(0.70)], Admit::Held),
        (vec![five(0.1), week(0.69)], Admit::Go),
    ] {
        e.limits.record("allowed", readings.clone()).await.unwrap();
        assert_eq!(e.limits.admit(&sched, 0), want, "{readings:?}");
    }
    for t in [
        TriggerKind::Watcher,
        TriggerKind::FollowUp,
        TriggerKind::Webhook,
    ] {
        e.limits.record("allowed", vec![five(0.9)]).await.unwrap();
        assert_eq!(e.limits.admit(&q("r", t), 0), Admit::Held, "{t:?}");
    }
}

#[tokio::test]
async fn the_switch_off_holds_nothing() {
    let e = env(HoldShares {
        enabled: false,
        ..SHARES
    })
    .await;
    e.limits.record("allowed", vec![five(0.99)]).await.unwrap();
    assert_eq!(e.limits.admit(&q("r", TriggerKind::Schedule), 0), Admit::Go);
}

#[tokio::test]
async fn run_anyway_goes() {
    let e = env(SHARES).await;
    e.limits.record("allowed", vec![five(0.9)]).await.unwrap();
    let mut run = q("r", TriggerKind::Schedule);
    run.hold_released = true;
    assert_eq!(e.limits.admit(&run, 0), Admit::Go);
}

#[tokio::test]
async fn rejected_stops_every_claude_run_until_it_resets() {
    let e = env(SHARES).await;
    let mut limited = five(1.0);
    limited.status = "rejected".into();
    e.limits.record("rejected", vec![limited]).await.unwrap();
    assert_eq!(
        e.limits.admit(&q("c", TriggerKind::Chat), 0),
        Admit::Rejected
    );
    let mut released = q("s", TriggerKind::Schedule);
    released.hold_released = true;
    assert_eq!(e.limits.admit(&released, 0), Admit::Rejected);
    assert_eq!(
        e.limits.rejected_until().as_deref(),
        Some(timestamp(t0() + TimeDelta::hours(3)).as_str())
    );
    e.clock
        .set(t0() + TimeDelta::hours(3) + TimeDelta::seconds(1));
    assert_eq!(e.limits.admit(&q("c", TriggerKind::Chat), 0), Admit::Go);
}

#[tokio::test]
async fn a_reading_already_reset_counts_as_reset() {
    let e = env(SHARES).await;
    e.limits
        .record(
            "allowed",
            vec![reading("five_hour", 0.9, TimeDelta::minutes(-1))],
        )
        .await
        .unwrap();
    let sched = q("r", TriggerKind::Schedule);
    assert_eq!(e.limits.admit(&sched, 0), Admit::Go);
    assert_eq!(e.limits.admit(&sched, 1), Admit::Held);
    assert_eq!(e.limits.state(&[], 0).five_hour, None);
}

#[tokio::test]
async fn a_window_never_read_does_not_keep_runs_one_at_a_time() {
    let e = env(SHARES).await;
    e.limits.record("allowed", vec![five(0.1)]).await.unwrap();
    assert_eq!(e.limits.admit(&q("r", TriggerKind::Schedule), 1), Admit::Go);
}

#[tokio::test]
async fn draining_lets_one_at_a_time_until_the_queue_is_empty() {
    let e = env(SHARES).await;
    let queued = [q("a", TriggerKind::Schedule), q("b", TriggerKind::Watcher)];
    e.limits.record("allowed", vec![five(0.7)]).await.unwrap();
    e.limits.publish(&queued, 0);
    assert!(e.limits.state(&queued, 0).holding);

    e.limits.record("allowed", vec![five(0.2)]).await.unwrap();
    e.limits.publish(&queued, 0);
    let state = e.limits.state(&queued, 0);
    assert!(!state.holding);
    assert!(state.draining);
    // The first may start; the second waits for it.
    assert_eq!(state.held_run_ids, ["b"]);
    assert_eq!(e.limits.admit(&queued[0], 0), Admit::Go);
    assert_eq!(e.limits.admit(&queued[1], 1), Admit::Held);

    e.limits.publish(&[], 0);
    assert!(!e.limits.state(&[], 0).draining);
    assert_eq!(e.limits.admit(&queued[1], 1), Admit::Go);
}

#[tokio::test]
async fn holding_start_and_end_are_published_once() {
    let e = env(SHARES).await;
    let mut rx = e.bus.subscribe();
    let queued = [q("a", TriggerKind::Schedule)];
    for u in [0.64, 0.66] {
        e.limits.record("allowed", vec![five(u)]).await.unwrap();
        e.limits.publish(&queued, 0);
    }
    e.limits.record("allowed", vec![five(0.2)]).await.unwrap();
    e.limits.publish(&queued, 0);

    let mut changes = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::LimitsUpdated { change, .. } = ev {
            changes.extend(change);
        }
    }
    assert_eq!(
        changes,
        [
            HoldChange::Started {
                window: "five_hour".into(),
                utilization: 0.64,
            },
            HoldChange::Ended,
        ]
    );
}

/// Nothing is sent until something changes: the dispatcher publishes on its first pass, and a
/// runtime with nothing to say says nothing.
#[tokio::test]
async fn the_state_at_load_is_not_published() {
    let e = env(SHARES).await;
    let mut rx = e.bus.subscribe();
    e.limits.publish(&[], 0);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn an_unchanged_state_is_published_once() {
    let e = env(SHARES).await;
    let mut rx = e.bus.subscribe();
    let queued = [q("a", TriggerKind::Schedule)];
    e.limits.record("allowed", vec![five(0.2)]).await.unwrap();
    for _ in 0..3 {
        e.limits.publish(&queued, 0);
    }
    e.limits.record("allowed", vec![five(0.3)]).await.unwrap();
    e.limits.publish(&queued, 0);
    let mut sent = 0;
    while let Ok(ev) = rx.try_recv() {
        if matches!(ev, RuntimeEvent::LimitsUpdated { .. }) {
            sent += 1;
        }
    }
    assert_eq!(sent, 2);
}

#[tokio::test]
async fn readings_survive_a_reload() {
    let e = env(SHARES).await;
    e.limits.record("allowed", vec![five(0.42)]).await.unwrap();
    let again = Limits::load(e.store.clone(), e.bus.clone(), e.clock.clone(), SHARES)
        .await
        .unwrap();
    assert_eq!(again.state(&[], 0).five_hour, Some(five(0.42)));
    assert_eq!(e.clock.now(), t0());
}
