//! The Usage page's numbers (usage and limits spec section 5): each dot's runs started since a
//! time, their tokens, run time and API-equivalent cost, heaviest dots first, with each dot's
//! heaviest runs.

mod common;

use chrono::{DateTime, TimeDelta, Utc};
use common::{spec, temp_store};
use dots_core::model::{Dot, EngineKind, NewRun, TriggerKind};
use dots_core::store::Store;
use dots_core::util::timestamp;

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-26T12:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc)
}

/// A run of `dot` that started `ago` before now, ran `secs`, and used `tokens` (in, out).
async fn run(
    store: &Store,
    dot: &Dot,
    ago: TimeDelta,
    secs: i64,
    tokens: (i64, i64),
    cost: Option<f64>,
) -> String {
    let r = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
        .await
        .unwrap();
    let started = now() - ago;
    store
        .set_run_times(
            &r.id,
            &timestamp(started),
            Some(&timestamp(started + TimeDelta::seconds(secs))),
        )
        .await
        .unwrap();
    store
        .add_usage(&r.id, tokens.0, tokens.1, 0, cost)
        .await
        .unwrap();
    r.id
}

#[tokio::test]
async fn the_usage_summary_sums_per_dot_in_the_period() {
    let (_d, store) = temp_store().await;
    let a = store.create_dot(&spec("a"), None, "default").await.unwrap();
    let mut local = spec("b");
    local.engine = EngineKind::Local;
    local.model = "m".into();
    local.endpoint_url = Some("http://127.0.0.1:1/v1".into());
    let b = store.create_dot(&local, None, "default").await.unwrap();

    let a1 = run(&store, &a, TimeDelta::hours(1), 60, (100, 10), Some(0.5)).await;
    let a2 = run(&store, &a, TimeDelta::days(3), 120, (1000, 100), Some(2.0)).await;
    run(&store, &a, TimeDelta::days(20), 30, (5, 5), None).await;
    let b1 = run(&store, &b, TimeDelta::minutes(30), 600, (2000, 500), None).await;
    // Not counted: before the month.
    run(&store, &a, TimeDelta::days(40), 30, (9, 9), Some(9.0)).await;

    let five = store
        .usage_rows(&timestamp(now() - TimeDelta::hours(5)))
        .await
        .unwrap();
    assert_eq!(five.len(), 2);
    assert_eq!(five[0].dot_name, "b");
    assert_eq!(five[0].engine, EngineKind::Local);
    assert_eq!(
        (five[0].runs, five[0].tokens_in, five[0].tokens_out),
        (1, 2000, 500)
    );
    assert_eq!((five[0].run_secs, five[0].cost_usd), (600, None));
    assert_eq!(five[0].heaviest.len(), 1);
    assert_eq!(five[0].heaviest[0].run_id, b1);
    assert_eq!(five[1].dot_name, "a");
    assert_eq!(
        (five[1].runs, five[1].run_secs, five[1].cost_usd),
        (1, 60, Some(0.5))
    );

    let week = store
        .usage_rows(&timestamp(now() - TimeDelta::days(7)))
        .await
        .unwrap();
    let a_row = week.iter().find(|r| r.dot_id == a.id).unwrap();
    assert_eq!(a_row.runs, 2);
    assert_eq!((a_row.tokens_in, a_row.tokens_out), (1100, 110));
    assert_eq!(a_row.run_secs, 180);
    assert_eq!(a_row.cost_usd, Some(2.5));
    let heaviest: Vec<&str> = a_row.heaviest.iter().map(|r| r.run_id.as_str()).collect();
    assert_eq!(heaviest, [a2.as_str(), a1.as_str()]);
    assert_eq!(a_row.heaviest[0].run_secs, Some(120));
    assert_eq!(a_row.heaviest[0].cost_usd, Some(2.0));
    // b's 2500 tokens are more than a's 1210: dots are sorted by tokens.
    assert_eq!(week[0].dot_name, "b");

    let month = store
        .usage_rows(&timestamp(now() - TimeDelta::days(30)))
        .await
        .unwrap();
    let a_row = month.iter().find(|r| r.dot_id == a.id).unwrap();
    assert_eq!(a_row.runs, 3);
    assert_eq!(a_row.cost_usd, Some(2.5));
}

#[tokio::test]
async fn at_most_five_heaviest_runs_and_unfinished_runs_add_no_time() {
    let (_d, store) = temp_store().await;
    let a = store.create_dot(&spec("a"), None, "default").await.unwrap();
    for n in 1..=7 {
        run(&store, &a, TimeDelta::minutes(n), 10, (n * 10, 0), None).await;
    }
    let open = store
        .create_run(&NewRun::new(&a.id, TriggerKind::Manual))
        .await
        .unwrap();
    store
        .set_run_times(&open.id, &timestamp(now() - TimeDelta::minutes(1)), None)
        .await
        .unwrap();
    let rows = store
        .usage_rows(&timestamp(now() - TimeDelta::hours(5)))
        .await
        .unwrap();
    assert_eq!(rows[0].runs, 8);
    assert_eq!(rows[0].run_secs, 70);
    assert_eq!(rows[0].heaviest.len(), 5);
    assert_eq!(rows[0].heaviest[0].tokens_in, 70);
    assert!(rows[0]
        .heaviest
        .windows(2)
        .all(|w| w[0].tokens_in + w[0].tokens_out >= w[1].tokens_in + w[1].tokens_out));
}
