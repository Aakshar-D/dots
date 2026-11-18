//! Spending caps (spend caps design): what a day and a month spent, read from the runs (their
//! estimated cost and tokens, a run still going with what it has reported); which cap is
//! reached and how it is named; the local day and month; and the caps' checks.

mod common;

use chrono::{DateTime, TimeDelta, Utc};
use common::tz::TestTz;
use common::{spec, temp_store};
use dots_core::model::{Dot, NewRun, TriggerKind};
use dots_core::settings::Settings;
use dots_core::spend::{
    day_bounds_in, first_hit, month_bounds_in, CapScope, DotCaps, GlobalCaps, Ledger, Spend,
};
use dots_core::store::Store;
use dots_core::util::timestamp;
use dots_core::Error;

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

const NONE: Spend = Spend {
    cost_usd: 0.0,
    tokens: 0,
};

fn spent(cost_usd: f64, tokens: i64) -> Spend {
    Spend { cost_usd, tokens }
}

/// A run of `dot` started at `at` that reported `tokens` (in, out) and `cost`; ended unless
/// `going`.
async fn run_at(
    store: &Store,
    dot: &Dot,
    at: DateTime<Utc>,
    tokens: (i64, i64),
    cost: Option<f64>,
    going: bool,
) -> String {
    let r = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
        .await
        .unwrap();
    let ended = (!going).then(|| timestamp(at + TimeDelta::minutes(1)));
    store
        .set_run_times(&r.id, &timestamp(at), ended.as_deref())
        .await
        .unwrap();
    store
        .add_usage(&r.id, tokens.0, tokens.1, 0, cost)
        .await
        .unwrap();
    r.id
}

#[test]
fn the_first_cap_reached_is_named() {
    let dot = DotCaps {
        daily_usd: Some(2.0),
        daily_tokens: Some(200_000),
    };
    let all = GlobalCaps {
        daily_usd: Some(10.0),
        daily_tokens: Some(1_000_000),
        monthly_usd: Some(50.0),
    };
    let hit = |d: Spend, t: Spend, m: Spend| first_hit("Scout", &dot, d, &all, t, m);
    assert_eq!(hit(spent(1.99, 199_999), NONE, NONE), None);

    let h = hit(spent(2.0, 0), NONE, NONE).unwrap();
    assert_eq!(
        (h.text.as_str(), h.scope),
        ("Scout reached its $2.00 daily cap", CapScope::Dot)
    );
    let h = hit(spent(0.5, 200_000), NONE, NONE).unwrap();
    assert_eq!(h.text, "Scout reached its 200,000-token daily cap");
    let h = hit(NONE, spent(10.5, 0), NONE).unwrap();
    assert_eq!(
        (h.text.as_str(), h.scope),
        ("All sols reached the $10.00 daily cap", CapScope::All)
    );
    let h = hit(NONE, spent(0.0, 1_000_000), NONE).unwrap();
    assert_eq!(h.text, "All sols reached the 1,000,000-token daily cap");
    let h = hit(NONE, NONE, spent(50.0, 0)).unwrap();
    assert_eq!(h.text, "All sols reached the $50.00 monthly cap");
    // The dot's own cap comes first.
    let h = hit(spent(3.0, 0), spent(20.0, 0), spent(80.0, 0)).unwrap();
    assert_eq!(h.scope, CapScope::Dot);

    // No cap, nothing reached however much was spent.
    let big = spent(1e9, i64::MAX);
    assert_eq!(
        first_hit(
            "Scout",
            &DotCaps::default(),
            big,
            &GlobalCaps::default(),
            big,
            big
        ),
        None
    );
}

#[test]
fn a_day_is_local_midnight_to_midnight() {
    // TestTz is UTC+1 in winter and UTC+2 in summer.
    let (start, end) = day_bounds_in(utc("2026-11-06T22:59:00Z"), &TestTz);
    assert_eq!(
        (start.as_str(), end.as_str()),
        ("2026-11-05T23:00:00.000Z", "2026-11-06T23:00:00.000Z")
    );
    // A minute later it is the next local day.
    let (start, _) = day_bounds_in(utc("2026-11-06T23:00:00Z"), &TestTz);
    assert_eq!(start, "2026-11-06T23:00:00.000Z");
    // The day summer time ends is 25 hours long.
    let (start, end) = day_bounds_in(utc("2026-10-25T12:00:00Z"), &TestTz);
    assert_eq!(
        (start.as_str(), end.as_str()),
        ("2026-10-24T22:00:00.000Z", "2026-10-25T23:00:00.000Z")
    );
}

#[test]
fn a_month_is_the_1st_to_the_next_1st() {
    let (start, end) = month_bounds_in(utc("2026-11-06T12:00:00Z"), &TestTz);
    assert_eq!(
        (start.as_str(), end.as_str()),
        ("2026-10-31T23:00:00.000Z", "2026-11-30T23:00:00.000Z")
    );
    let (start, end) = month_bounds_in(utc("2026-12-31T22:30:00Z"), &TestTz);
    assert_eq!(
        (start.as_str(), end.as_str()),
        ("2026-11-30T23:00:00.000Z", "2026-12-31T23:00:00.000Z")
    );
    // Past local midnight on New Year's Eve it is January.
    let (start, end) = month_bounds_in(utc("2026-12-31T23:30:00Z"), &TestTz);
    assert_eq!(
        (start.as_str(), end.as_str()),
        ("2026-12-31T23:00:00.000Z", "2027-01-31T23:00:00.000Z")
    );
}

#[tokio::test]
async fn a_days_spend_counts_its_runs_including_one_still_going() {
    let (_d, store) = temp_store().await;
    let a = store.create_dot(&spec("a"), None, "default").await.unwrap();
    let b = store.create_dot(&spec("b"), None, "default").await.unwrap();
    let (start, end) = day_bounds_in(utc("2026-11-06T12:00:00Z"), &TestTz);
    let midnight = utc(&start);

    run_at(&store, &a, midnight, (100, 10), Some(0.5), false).await;
    // Still going: what it reported so far counts.
    run_at(
        &store,
        &a,
        midnight + TimeDelta::hours(10),
        (1000, 100),
        Some(1.25),
        true,
    )
    .await;
    // A local run reports no cost; its tokens count.
    run_at(
        &store,
        &b,
        midnight + TimeDelta::hours(1),
        (40, 2),
        None,
        false,
    )
    .await;
    // Yesterday, a minute before midnight: not today.
    run_at(
        &store,
        &a,
        midnight - TimeDelta::minutes(1),
        (9, 9),
        Some(9.0),
        false,
    )
    .await;
    // Tomorrow's first minute: not today either.
    run_at(&store, &a, utc(&end), (7, 7), Some(7.0), false).await;
    // Queued, not started: it spent nothing.
    store
        .create_run(&NewRun::new(&a.id, TriggerKind::Manual))
        .await
        .unwrap();

    let of_a = store
        .spend_between(Some(&a.id), &start, &end)
        .await
        .unwrap();
    assert_eq!(of_a, spent(1.75, 1210));
    let of_b = store
        .spend_between(Some(&b.id), &start, &end)
        .await
        .unwrap();
    assert_eq!(of_b, spent(0.0, 42));
    let all = store.spend_between(None, &start, &end).await.unwrap();
    assert_eq!(all, spent(1.75, 1252));

    let mut by_dot = store.spend_by_dot(&start, &end).await.unwrap();
    by_dot.sort_by(|x, y| x.0.cmp(&y.0));
    let mut want = vec![(a.id.clone(), of_a), (b.id.clone(), of_b)];
    want.sort_by(|x, y| x.0.cmp(&y.0));
    assert_eq!(by_dot, want);
}

#[tokio::test]
async fn the_ledger_reads_todays_spend_against_the_caps() {
    let (_d, store) = temp_store().await;
    let mut capped = spec("scout");
    capped.spend_cap = DotCaps {
        daily_usd: Some(1.0),
        daily_tokens: None,
    };
    let scout = store.create_dot(&capped, None, "default").await.unwrap();
    let other = store
        .create_dot(&spec("other"), None, "default")
        .await
        .unwrap();
    let now = Utc::now();
    let settings = Settings::default();

    let mut ledger = Ledger::new(&settings, now);
    assert_eq!(ledger.hit(&store, &scout).await.unwrap(), None);

    run_at(&store, &scout, now, (10, 10), Some(1.0), true).await;
    let mut ledger = Ledger::new(&settings, now);
    let hit = ledger.hit(&store, &scout).await.unwrap().unwrap();
    assert_eq!(hit.text, "scout reached its $1.00 daily cap");
    // Another dot, with no cap of its own, is not held by it.
    assert_eq!(ledger.hit(&store, &other).await.unwrap(), None);

    // The cap of every dot together holds every dot.
    let all = Settings {
        spend_daily_tokens: Some(20),
        ..Settings::default()
    };
    let mut ledger = Ledger::new(&all, now);
    let hit = ledger.hit(&store, &other).await.unwrap().unwrap();
    assert_eq!(
        (hit.text.as_str(), hit.scope),
        ("All sols reached the 20-token daily cap", CapScope::All)
    );
    // A dot that is gone has no cap.
    assert_eq!(ledger.hit_for(&store, "no-such-dot").await.unwrap(), None);
}

#[tokio::test]
async fn caps_are_stored_with_the_dot_and_checked() {
    let (_d, store) = temp_store().await;
    let mut s = spec("scout");
    s.spend_cap = DotCaps {
        daily_usd: Some(2.5),
        daily_tokens: Some(300_000),
    };
    let dot = store.create_dot(&s, None, "default").await.unwrap();
    assert_eq!(
        store.get_dot(&dot.id).await.unwrap().spec.spend_cap,
        s.spend_cap
    );

    s.spend_cap = DotCaps::default();
    let dot = store.update_dot(&dot.id, &s).await.unwrap();
    assert_eq!(dot.spec.spend_cap, DotCaps::default());

    for caps in [
        DotCaps {
            daily_usd: Some(0.0),
            daily_tokens: None,
        },
        DotCaps {
            daily_usd: Some(-1.0),
            daily_tokens: None,
        },
        DotCaps {
            daily_usd: Some(100_001.0),
            daily_tokens: None,
        },
        DotCaps {
            daily_usd: None,
            daily_tokens: Some(0),
        },
    ] {
        s.spend_cap = caps;
        let err = store.update_dot(&dot.id, &s).await.unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{caps:?}: {err}");
    }
}

#[tokio::test]
async fn settings_caps_are_stored_and_checked() {
    let (_d, store) = temp_store().await;
    let caps = Settings {
        spend_daily_usd: Some(10.0),
        spend_daily_tokens: Some(2_000_000),
        spend_monthly_usd: Some(150.0),
        ..Settings::default()
    };
    store.put_settings(&caps).await.unwrap();
    assert_eq!(store.get_settings().await.unwrap(), caps);

    for bad in [
        Settings {
            spend_daily_usd: Some(0.001),
            ..Settings::default()
        },
        Settings {
            spend_monthly_usd: Some(f64::INFINITY),
            ..Settings::default()
        },
        Settings {
            spend_daily_tokens: Some(0),
            ..Settings::default()
        },
    ] {
        assert!(matches!(
            store.put_settings(&bad).await.unwrap_err(),
            Error::Invalid(_)
        ));
    }
    // Clearing a cap is allowed.
    store.put_settings(&Settings::default()).await.unwrap();
    assert_eq!(store.get_settings().await.unwrap().spend_daily_usd, None);
}

#[test]
fn a_spec_without_caps_reads_as_none() {
    let mut v = serde_json::to_value(spec("old")).unwrap();
    v.as_object_mut().unwrap().remove("spend_cap");
    let s: dots_core::model::DotSpec = serde_json::from_value(v).unwrap();
    assert_eq!(s.spend_cap, DotCaps::default());
}
