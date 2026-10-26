//! The org timers (env vault spec §6.4): each joined org is fetched every `refresh_minutes` of
//! its accepted policy, a policy that changes the interval re-times the org from its next
//! fetch, and **Refresh** or **Update** count from now. A stopped timer fetches no more, and
//! every timer and the start-up fetch stop with the shutdown token. The clock is the fake the
//! scheduler's tests use: a timer sleeps at most a minute at a time, and a test knows the timer
//! is idle once it sleeps until the deadline the test expects.

mod common;

use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use common::clock::FakeClock;
use common::fake_openbao::Failure;
use common::orgs::{at, policy, sign, token, OrgsHarness, ACME, START};
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::vault::org::OrgTimers;
use serde_json::json;
use tokio::sync::broadcast::Receiver;
use tokio_util::sync::CancellationToken;

const ORG_VALUE: &str = "org-secret-timer-value-5b2c71";

/// `START` and `minutes` more.
fn plus(minutes: i64) -> DateTime<Utc> {
    at(START) + Duration::minutes(minutes)
}

/// Waits (at most 10 real seconds) until the newest sleep of `clock` has `deadline`: the timer
/// has done what the time asked of it and waits again.
async fn sleeping_until(clock: &FakeClock, deadline: DateTime<Utc>) {
    let waited = tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            let n = clock.sleeps();
            if n > 0 && clock.nth_sleep(n).await == deadline {
                return;
            }
            tokio::time::sleep(StdDuration::from_millis(5)).await;
        }
    })
    .await;
    assert!(waited.is_ok(), "no timer began to sleep until {deadline}");
}

/// Waits (at most 10 real seconds) for `check` to hold.
async fn eventually(what: &str, check: impl Fn() -> bool) {
    for _ in 0..1000 {
        if check() {
            return;
        }
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
    panic!("{what} did not happen");
}

/// The next `OrgUpdated`'s org, skipping other events.
async fn next_org_updated(events: &mut Receiver<RuntimeEvent>) -> String {
    let next = tokio::time::timeout(StdDuration::from_secs(10), async {
        loop {
            if let RuntimeEvent::OrgUpdated { org_id } = events.recv().await.unwrap() {
                return org_id;
            }
        }
    });
    next.await.expect("no OrgUpdated was published")
}

/// Acme joined at `START` with a policy of `extra`, and timers on the harness's clock.
async fn joined(extra: serde_json::Value) -> (OrgsHarness, OrgTimers, Bus, CancellationToken) {
    let h = OrgsHarness::new().await;
    h.join_acme(json!({"API_KEY": ORG_VALUE}), extra).await;
    let bus = new_bus();
    let shutdown = CancellationToken::new();
    let timers = OrgTimers::new(
        h.orgs.clone(),
        h.clock.clone(),
        bus.clone(),
        shutdown.clone(),
    );
    (h, timers, bus, shutdown)
}

#[tokio::test]
async fn an_org_is_fetched_every_refresh_minutes_of_its_policy() {
    let (h, timers, bus, _shutdown) = joined(json!({})).await;
    let mut events = bus.subscribe();
    timers.start(ACME);
    assert_eq!(timers.running(), [ACME]);
    // At most a minute at a time, as the scheduler sleeps.
    sleeping_until(&h.clock, plus(1)).await;
    h.clock.set(plus(14) + Duration::seconds(59));
    sleeping_until(&h.clock, plus(15)).await;
    assert_eq!(h.env_reads("dots"), 1, "only the join's fetch so far");

    h.clock.set(plus(15));
    sleeping_until(&h.clock, plus(16)).await;
    assert_eq!(h.env_reads("dots"), 2);
    assert_eq!(next_org_updated(&mut events).await, ACME);
    h.clock.set(plus(30));
    sleeping_until(&h.clock, plus(31)).await;
    assert_eq!(h.env_reads("dots"), 3);
    assert_eq!(next_org_updated(&mut events).await, ACME);
}

/// A fetch that brings a policy with another `refresh_minutes` re-times the org from there.
#[tokio::test]
async fn a_policy_that_changes_the_interval_retimes_the_org_at_its_next_fetch() {
    let (h, timers, _bus, _shutdown) = joined(json!({})).await;
    timers.start(ACME);
    sleeping_until(&h.clock, plus(1)).await;
    let every_five = json!({"refresh_minutes": 5});
    let signed = sign(&h.keys, &policy(ACME, "2026-10-22T08:30:00Z", every_five));
    h.serve("dots", json!({"API_KEY": ORG_VALUE}), &signed);

    h.clock.set(plus(15));
    sleeping_until(&h.clock, plus(16)).await;
    assert_eq!(h.env_reads("dots"), 2);
    assert_eq!(h.orgs.policy(ACME).unwrap().refresh_minutes, 5);
    h.clock.set(plus(19) + Duration::seconds(59));
    sleeping_until(&h.clock, plus(20)).await;
    assert_eq!(h.env_reads("dots"), 2, "not before five minutes");
    h.clock.set(plus(20));
    sleeping_until(&h.clock, plus(21)).await;
    assert_eq!(h.env_reads("dots"), 3);
}

/// **Refresh** and **Update** fetch now, so the next fetch counts from now. Starting a running
/// timer again keeps that one timer, re-timed.
#[tokio::test]
async fn a_refresh_counts_from_now_and_a_second_start_keeps_one_timer() {
    let (h, timers, _bus, _shutdown) = joined(json!({})).await;
    timers.start(ACME);
    sleeping_until(&h.clock, plus(1)).await;
    h.clock.set(plus(10));
    sleeping_until(&h.clock, plus(11)).await;
    h.orgs.refresh(ACME).await.unwrap();
    // The new time holds from the call on, whenever the timer's task next runs.
    timers.retime(ACME);
    timers.start(ACME);
    assert_eq!(timers.running(), [ACME]);

    h.clock.set(plus(15));
    sleeping_until(&h.clock, plus(16)).await;
    assert_eq!(
        h.env_reads("dots"),
        2,
        "the refresh, and no timer fetch at 15 minutes"
    );
    h.clock.set(plus(24) + Duration::seconds(59));
    sleeping_until(&h.clock, plus(25)).await;
    assert_eq!(h.env_reads("dots"), 2);
    h.clock.set(plus(25));
    sleeping_until(&h.clock, plus(26)).await;
    assert_eq!(h.env_reads("dots"), 3, "one fetch: one timer");
}

#[tokio::test]
async fn a_stopped_timer_fetches_no_more() {
    let (h, timers, _bus, _shutdown) = joined(json!({})).await;
    timers.start(ACME);
    sleeping_until(&h.clock, plus(1)).await;
    timers.stop(ACME);
    eventually("the timer's end", || timers.running().is_empty()).await;
    let sleeps = h.clock.sleeps();
    h.clock.set(plus(60));
    tokio::time::sleep(StdDuration::from_millis(200)).await;
    assert_eq!(h.env_reads("dots"), 1);
    assert_eq!(h.clock.sleeps(), sleeps, "no timer sleeps again");
    // A stopped org can be started again.
    timers.start(ACME);
    assert_eq!(timers.running(), [ACME]);
}

/// An org removed without its timer being stopped ends the timer at its next fetch.
#[tokio::test]
async fn a_timer_ends_when_its_org_is_no_longer_joined() {
    let (h, timers, _bus, _shutdown) = joined(json!({})).await;
    timers.start(ACME);
    sleeping_until(&h.clock, plus(1)).await;
    h.orgs.remove(ACME).await.unwrap();
    h.clock.set(plus(15));
    eventually("the timer's end", || timers.running().is_empty()).await;
    assert_eq!(h.env_reads("dots"), 1);
}

/// The start-up fetch starts every joined org's timer, fetches each org once, and says so.
#[tokio::test]
async fn the_start_up_fetch_fetches_every_org_once_and_starts_its_timer() {
    let (h, timers, bus, _shutdown) = joined(json!({})).await;
    let signed = sign(&h.keys, &policy("beta", "2026-10-22T08:00:00Z", json!({})));
    h.serve("beta", json!({"API_KEY": ORG_VALUE}), &signed);
    h.orgs
        .join(&h.code_at("beta", "Beta", "beta"), Some(token()))
        .await
        .unwrap();
    let mut events = bus.subscribe();
    timers.start_up();
    let mut said = vec![
        next_org_updated(&mut events).await,
        next_org_updated(&mut events).await,
    ];
    said.sort();
    assert_eq!(said, [ACME, "beta"]);
    assert_eq!((h.env_reads("dots"), h.env_reads("beta")), (2, 2));
    assert_eq!(timers.running(), [ACME, "beta"]);
    eventually("the start-up fetch's end", || !timers.starting_up()).await;
}

/// Every timer, and a start-up fetch that hangs, stop with the shutdown token.
#[tokio::test]
async fn the_timers_and_the_start_up_fetch_stop_with_the_shutdown_token() {
    let (h, timers, _bus, shutdown) = joined(json!({})).await;
    h.bao.fail_next(Failure::Hang, 1);
    timers.start_up();
    eventually("the timer's start", || timers.running() == [ACME]).await;
    eventually("the hanging fetch", || h.env_reads("dots") == 2).await;
    assert!(timers.starting_up());
    shutdown.cancel();
    eventually("the end of every task", || {
        timers.running().is_empty() && !timers.starting_up()
    })
    .await;
}
