//! Shared spend (dots-make-dots design section 6, Spend; proof 10 of section 13): what a made dot
//! spends counts toward the daily caps of its maker, and of that dot's maker, up the chain, as
//! well as its own; a reached cap holds and stops the runs of the maker and of every dot below
//! it as a capped dot's runs are held and stopped; and the spend of a deleted dot still counts
//! toward its makers for the rest of the day, and toward every dot together for the month.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, TimeDelta, Utc};
use common::{spec, temp_store, wait_status};
use dots_core::engine::scripted::{ScriptedEngine, Step};
use dots_core::engine::{Engine, EngineEvent};
use dots_core::events::new_bus;
use dots_core::making::reach::Wanted;
use dots_core::making::service::{MakeDot, MakeOutcome};
use dots_core::model::{
    Dot, DotSpec, EngineKind, Made, NewRun, Run, RunStatus, TriggerKind, WaitReason, WorkspaceMode,
};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::settings::Settings;
use dots_core::spend::{
    day_bounds_in, month_bounds_in, CapScope, DotCaps, DotSpend, Ledger, Spend, SHARED_WITH_MADE,
};
use dots_core::store::Store;
use dots_core::util::timestamp;
use dots_core::vault::MemorySecrets;
use dots_core::{Config, Error, Runtime};
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn spent(cost_usd: f64, tokens: i64) -> Spend {
    Spend { cost_usd, tokens }
}

const NONE: Spend = Spend {
    cost_usd: 0.0,
    tokens: 0,
};

fn usd_cap(daily_usd: f64) -> DotCaps {
    DotCaps {
        daily_usd: Some(daily_usd),
        daily_tokens: None,
    }
}

/// Who made a dot `maker` made.
fn made_by(maker: &Dot) -> Made {
    Made {
        by_dot_id: Some(maker.id.clone()),
        by_name: maker.spec.name.clone(),
        run_id: "run".into(),
        purpose: "Reads the feeds".into(),
    }
}

/// A dot a person made, with `caps`.
async fn dot(store: &Store, name: &str, caps: DotCaps) -> Dot {
    let mut s = spec(name);
    s.spend_cap = caps;
    store.create_dot(&s, None, "default").await.unwrap()
}

/// A dot `maker` made, with `caps`.
async fn made(store: &Store, maker: &Dot, name: &str, caps: DotCaps) -> Dot {
    let mut s = spec(name);
    s.spend_cap = caps;
    store
        .create_made_dot(&s, None, "default", &made_by(maker))
        .await
        .unwrap()
}

/// A finished run of `dot` started at `at` that reported `tokens` (in + out) and cost `cost`.
async fn run_at(store: &Store, dot: &Dot, at: DateTime<Utc>, cost: f64, tokens: i64) -> Run {
    let r = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Schedule))
        .await
        .unwrap();
    store
        .set_run_times(&r.id, &timestamp(at), Some(&timestamp(at)))
        .await
        .unwrap();
    store
        .finish_run(&r.id, RunStatus::Succeeded, Some("ok"), None)
        .await
        .unwrap();
    store
        .add_usage(&r.id, tokens, 0, 0, Some(cost))
        .await
        .unwrap();
    store.get_run(&r.id).await.unwrap()
}

/// The words of the first cap reached for a run of `dot` at `now`, with the caps of the stored
/// Settings.
async fn hit(store: &Store, dot: &Dot, now: DateTime<Utc>) -> Option<String> {
    let settings = store.get_settings().await.unwrap();
    Ledger::new(&settings, now)
        .hit_for(store, &dot.id)
        .await
        .unwrap()
        .map(|h| h.text)
}

/// What counts toward the caps of `dot` on the local day of `now`.
async fn shared(store: &Store, dot: &Dot, now: DateTime<Utc>) -> Spend {
    let (start, end) = day_bounds_in(now, &Local);
    store
        .shared_spend_between(&dot.id, &start, &end)
        .await
        .unwrap()
}

/// The first moment of the local day after the one of `now`.
fn next_day(now: DateTime<Utc>) -> DateTime<Utc> {
    utc(&day_bounds_in(now, &Local).1)
}

/// The first moment of the local month after the one of `now`.
fn next_month(now: DateTime<Utc>) -> DateTime<Utc> {
    utc(&month_bounds_in(now, &Local).1)
}

/// The row of `dot` in today's spend.
fn row<'a>(dots: &'a [DotSpend], dot: &Dot) -> &'a DotSpend {
    dots.iter()
        .find(|d| d.dot_id == dot.id)
        .unwrap_or_else(|| panic!("{} is not in {dots:#?}", dot.spec.name))
}

// The ledger: whose cap is reached, and how it is named.

#[tokio::test]
async fn a_made_dots_spend_reaches_its_makers_cap_for_all_of_them() {
    let (_d, store) = temp_store().await;
    let maker = dot(&store, "Researcher", usd_cap(2.0)).await;
    let one = made(&store, &maker, "Blog Watcher", DotCaps::default()).await;
    let two = made(&store, &maker, "Feed Reader", DotCaps::default()).await;
    let other = dot(&store, "Other", DotCaps::default()).await;
    let now = Utc::now();
    assert_eq!(hit(&store, &maker, now).await, None);
    assert_eq!(hit(&store, &one, now).await, None);

    run_at(&store, &one, now, 1.5, 100).await;
    // Under the cap: the maker's own spend comes on top.
    assert_eq!(hit(&store, &maker, now).await, None);
    assert_eq!(shared(&store, &maker, now).await, spent(1.5, 100));
    run_at(&store, &maker, now, 0.5, 10).await;
    assert_eq!(shared(&store, &maker, now).await, spent(2.0, 110));
    // What a made dot spent is its own alone: its maker's and its sibling's are not below it.
    assert_eq!(shared(&store, &one, now).await, spent(1.5, 100));
    assert_eq!(shared(&store, &two, now).await, NONE);

    // The maker's own run reads as any capped dot's does.
    assert_eq!(
        hit(&store, &maker, now).await.as_deref(),
        Some("Researcher reached its $2.00 daily cap")
    );
    // A made dot's run names the maker, whose cap it is: the one that spent, and its sibling.
    let theirs = "Researcher reached its $2.00 daily cap, shared with the dots it made";
    assert_eq!(hit(&store, &one, now).await.as_deref(), Some(theirs));
    assert_eq!(hit(&store, &two, now).await.as_deref(), Some(theirs));
    assert_eq!(
        theirs,
        format!("Researcher reached its $2.00 daily cap{SHARED_WITH_MADE}")
    );
    // It is a dot's cap, not the one for every dot.
    let settings = Settings::default();
    let held = Ledger::new(&settings, now)
        .hit(&store, &one)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(held.scope, CapScope::Dot);
    // A dot outside the tree is not held.
    assert_eq!(hit(&store, &other, now).await, None);
    // Tomorrow nothing of it counts.
    assert_eq!(hit(&store, &one, next_day(now)).await, None);
    assert_eq!(hit(&store, &maker, next_day(now)).await, None);
}

#[tokio::test]
async fn a_made_dots_own_cap_holds_it_alone() {
    let (_d, store) = temp_store().await;
    let maker = dot(&store, "Researcher", usd_cap(10.0)).await;
    let one = made(&store, &maker, "Blog Watcher", usd_cap(1.0)).await;
    let two = made(&store, &maker, "Feed Reader", DotCaps::default()).await;
    let now = Utc::now();
    // A made dot starts with no cap; this one was given its own by the person.
    assert_eq!(two.spec.spend_cap, DotCaps::default());

    run_at(&store, &one, now, 1.0, 100).await;
    // Its own cap, in the words of any capped dot.
    assert_eq!(
        hit(&store, &one, now).await.as_deref(),
        Some("Blog Watcher reached its $1.00 daily cap")
    );
    assert_eq!(hit(&store, &maker, now).await, None);
    assert_eq!(hit(&store, &two, now).await, None);
    // The maker's spend does not count toward the made dot's cap.
    let other = made(&store, &maker, "Capped", usd_cap(1.0)).await;
    run_at(&store, &maker, now, 5.0, 100).await;
    assert_eq!(hit(&store, &other, now).await, None);
    assert_eq!(shared(&store, &other, now).await, NONE);
}

#[tokio::test]
async fn spend_counts_up_the_whole_chain() {
    let (_d, store) = temp_store().await;
    let a = dot(&store, "A", usd_cap(5.0)).await;
    let b = made(&store, &a, "B", usd_cap(3.0)).await;
    let c = made(&store, &b, "C", DotCaps::default()).await;
    let now = Utc::now();
    assert_eq!(
        store
            .makers_of(&c.id)
            .await
            .unwrap()
            .iter()
            .map(|d| d.spec.name.as_str())
            .collect::<Vec<_>>(),
        ["B", "A"]
    );
    assert!(store.makers_of(&a.id).await.unwrap().is_empty());

    run_at(&store, &c, now, 3.0, 100).await;
    assert_eq!(shared(&store, &b, now).await, spent(3.0, 100));
    assert_eq!(shared(&store, &a, now).await, spent(3.0, 100));
    // B's cap is reached by what C spent; A's is not yet.
    assert_eq!(
        hit(&store, &c, now).await.as_deref(),
        Some("B reached its $3.00 daily cap, shared with the dots it made")
    );
    assert_eq!(
        hit(&store, &b, now).await.as_deref(),
        Some("B reached its $3.00 daily cap")
    );
    assert_eq!(hit(&store, &a, now).await, None);

    run_at(&store, &c, now, 2.0, 100).await;
    assert_eq!(shared(&store, &a, now).await, spent(5.0, 200));
    assert_eq!(
        hit(&store, &a, now).await.as_deref(),
        Some("A reached its $5.00 daily cap")
    );
    // The nearest maker's cap is named first.
    assert_eq!(
        hit(&store, &c, now).await.as_deref(),
        Some("B reached its $3.00 daily cap, shared with the dots it made")
    );
    // Without B's cap, C and B are held by A's, two makers up for C.
    let mut s = b.spec.clone();
    s.spend_cap = DotCaps::default();
    store.update_dot(&b.id, &s).await.unwrap();
    let of_a = "A reached its $5.00 daily cap, shared with the dots it made";
    assert_eq!(hit(&store, &c, now).await.as_deref(), Some(of_a));
    assert_eq!(hit(&store, &b, now).await.as_deref(), Some(of_a));
}

#[tokio::test]
async fn the_token_cap_is_shared_as_the_cost_cap_is() {
    let (_d, store) = temp_store().await;
    let caps = DotCaps {
        daily_usd: None,
        daily_tokens: Some(1000),
    };
    let maker = dot(&store, "Researcher", caps).await;
    let one = made(&store, &maker, "Blog Watcher", DotCaps::default()).await;
    let now = Utc::now();
    run_at(&store, &one, now, 0.0, 600).await;
    assert_eq!(hit(&store, &one, now).await, None);
    run_at(&store, &maker, now, 0.0, 400).await;
    assert_eq!(
        hit(&store, &maker, now).await.as_deref(),
        Some("Researcher reached its 1,000-token daily cap")
    );
    assert_eq!(
        hit(&store, &one, now).await.as_deref(),
        Some("Researcher reached its 1,000-token daily cap, shared with the dots it made")
    );
}

#[tokio::test]
async fn a_dot_that_made_nothing_and_has_no_maker_reads_as_before() {
    let (_d, store) = temp_store().await;
    let caps = DotCaps {
        daily_usd: Some(2.0),
        daily_tokens: Some(200_000),
    };
    let scout = dot(&store, "Scout", caps).await;
    let now = Utc::now();
    let (start, end) = day_bounds_in(now, &Local);
    run_at(&store, &scout, now, 1.25, 1210).await;
    run_at(&store, &scout, now, 0.5, 42).await;

    let own = store
        .spend_between(Some(&scout.id), &start, &end)
        .await
        .unwrap();
    assert_eq!(own, spent(1.75, 1252));
    assert_eq!(shared(&store, &scout, now).await, own);
    assert_eq!(hit(&store, &scout, now).await, None);
    run_at(&store, &scout, now, 0.25, 0).await;
    assert_eq!(
        hit(&store, &scout, now).await.as_deref(),
        Some("Scout reached its $2.00 daily cap")
    );

    let today = dots_core::spend::today(&store, now).await.unwrap();
    assert_eq!(
        today.dots,
        [DotSpend {
            dot_id: scout.id.clone(),
            dot_name: "Scout".into(),
            today: spent(2.0, 1252),
            caps,
            shared_today: None,
        }]
    );
    assert_eq!(today.today, spent(2.0, 1252));
    // Deleted by the person, its spend goes with its runs, as before.
    store.delete_dot_if_idle(&scout.id).await.unwrap();
    let today = dots_core::spend::today(&store, now).await.unwrap();
    assert_eq!((today.today, today.month), (NONE, NONE));
    assert_eq!(
        store.kept_spend_between(None, &start, &end).await.unwrap(),
        NONE
    );
}

// What a deleted dot spent.

#[tokio::test]
async fn a_deleted_made_dots_spend_still_counts_for_the_day() {
    let (_d, store) = temp_store().await;
    let maker = dot(&store, "Researcher", usd_cap(2.0)).await;
    let one = made(&store, &maker, "Blog Watcher", DotCaps::default()).await;
    let two = made(&store, &maker, "Feed Reader", DotCaps::default()).await;
    let now = Utc::now();
    let (start, end) = day_bounds_in(now, &Local);
    run_at(&store, &one, now, 1.5, 100).await;
    run_at(&store, &one, now, 0.5, 20).await;
    // A run that spent nothing, and one of the month before, leave nothing behind.
    run_at(&store, &one, now, 0.0, 0).await;
    let last_month = utc(&month_bounds_in(now, &Local).0) - TimeDelta::minutes(1);
    run_at(&store, &one, last_month, 9.0, 900).await;
    let before = shared(&store, &maker, now).await;
    assert_eq!(before, spent(2.0, 120));

    store.delete_dot_if_idle(&one.id).await.unwrap();

    assert_eq!(shared(&store, &maker, now).await, before);
    assert_eq!(
        store
            .kept_spend_between(Some(&maker.id), &start, &end)
            .await
            .unwrap(),
        before
    );
    assert_eq!(
        hit(&store, &maker, now).await.as_deref(),
        Some("Researcher reached its $2.00 daily cap")
    );
    assert_eq!(
        hit(&store, &two, now).await.as_deref(),
        Some("Researcher reached its $2.00 daily cap, shared with the dots it made")
    );
    // The next day it counts no more.
    assert_eq!(shared(&store, &maker, next_day(now)).await, NONE);
    assert_eq!(hit(&store, &maker, next_day(now)).await, None);
    assert_eq!(hit(&store, &two, next_day(now)).await, None);
    // Nothing older than this month was kept.
    assert_eq!(
        store
            .kept_spend_between(Some(&maker.id), "2000-01-01T00:00:00.000Z", &start)
            .await
            .unwrap(),
        NONE
    );

    // What was kept for the maker goes with the maker.
    store.delete_dot_if_idle(&two.id).await.unwrap();
    store.delete_dot_if_idle(&maker.id).await.unwrap();
    assert_eq!(
        store
            .kept_spend_between(Some(&maker.id), &start, &end)
            .await
            .unwrap(),
        NONE
    );
}

#[tokio::test]
async fn deleting_the_middle_of_a_chain_keeps_what_it_and_its_deleted_dot_spent() {
    let (_d, store) = temp_store().await;
    let a = dot(&store, "A", usd_cap(3.0)).await;
    let b = made(&store, &a, "B", usd_cap(2.0)).await;
    let c = made(&store, &b, "C", DotCaps::default()).await;
    let now = Utc::now();
    run_at(&store, &c, now, 1.0, 100).await;
    run_at(&store, &b, now, 0.5, 10).await;

    store.delete_dot_if_idle(&c.id).await.unwrap();
    // It counts toward both makers above it.
    assert_eq!(shared(&store, &b, now).await, spent(1.5, 110));
    assert_eq!(shared(&store, &a, now).await, spent(1.5, 110));

    store.delete_dot_if_idle(&b.id).await.unwrap();
    // What B held for C and what B spent itself still count toward A, once each.
    assert_eq!(shared(&store, &a, now).await, spent(1.5, 110));
    run_at(&store, &a, now, 1.5, 0).await;
    assert_eq!(
        hit(&store, &a, now).await.as_deref(),
        Some("A reached its $3.00 daily cap")
    );
}

#[tokio::test]
async fn deleting_a_maker_keeps_what_the_dots_below_it_spent_so_far() {
    let (_d, store) = temp_store().await;
    let a = dot(&store, "A", usd_cap(3.0)).await;
    let b = made(&store, &a, "B", DotCaps::default()).await;
    let c = made(&store, &b, "C", DotCaps::default()).await;
    let now = Utc::now();
    run_at(&store, &c, now, 1.0, 100).await;
    run_at(&store, &b, now, 0.5, 10).await;
    assert_eq!(shared(&store, &a, now).await, spent(1.5, 110));

    // B goes while C lives: A cannot shed what C spent by deleting the dot between them.
    store.delete_dot_if_idle(&b.id).await.unwrap();
    assert_eq!(shared(&store, &a, now).await, spent(1.5, 110));
    // C has no maker now: what it spends from here on is its own alone.
    let c_now = store.get_dot(&c.id).await.unwrap();
    assert_eq!(c_now.made.unwrap().by_dot_id, None);
    run_at(&store, &c, now, 4.0, 400).await;
    assert_eq!(shared(&store, &a, now).await, spent(1.5, 110));
    assert_eq!(shared(&store, &c, now).await, spent(5.0, 500));
    assert_eq!(hit(&store, &c, now).await, None);
}

#[tokio::test]
async fn a_deleted_made_dots_spend_still_counts_for_every_dot_together() {
    let (_d, store) = temp_store().await;
    store
        .put_settings(&Settings {
            spend_daily_usd: Some(2.0),
            spend_monthly_usd: Some(2.0),
            ..Settings::default()
        })
        .await
        .unwrap();
    let maker = dot(&store, "Researcher", DotCaps::default()).await;
    let b = made(&store, &maker, "B", DotCaps::default()).await;
    let c = made(&store, &b, "C", DotCaps::default()).await;
    let other = dot(&store, "Other", DotCaps::default()).await;
    let now = Utc::now();
    run_at(&store, &c, now, 2.0, 100).await;
    let before = dots_core::spend::today(&store, now).await.unwrap();
    assert_eq!(
        (before.today, before.month),
        (spent(2.0, 100), spent(2.0, 100))
    );

    // Two makers above it: it is counted once for every dot together.
    store.delete_dot_if_idle(&c.id).await.unwrap();

    let after = dots_core::spend::today(&store, now).await.unwrap();
    assert_eq!((after.today, after.month), (before.today, before.month));
    assert_eq!(
        hit(&store, &other, now).await.as_deref(),
        Some("All dots reached the $2.00 daily cap")
    );
    // The next day the daily cap is free; the monthly one is not, until the month ends.
    let tomorrow = next_day(now);
    if tomorrow < next_month(now) {
        assert_eq!(
            hit(&store, &other, tomorrow).await.as_deref(),
            Some("All dots reached the $2.00 monthly cap")
        );
        let then = dots_core::spend::today(&store, tomorrow).await.unwrap();
        assert_eq!((then.today, then.month), (NONE, spent(2.0, 100)));
    }
    assert_eq!(hit(&store, &other, next_month(now)).await, None);
    // It stays counted when its makers go too.
    store.delete_dot_if_idle(&b.id).await.unwrap();
    store.delete_dot_if_idle(&maker.id).await.unwrap();
    let after = dots_core::spend::today(&store, now).await.unwrap();
    assert_eq!((after.today, after.month), (before.today, before.month));
}

#[tokio::test]
async fn todays_spend_shows_what_a_maker_shares() {
    let (_d, store) = temp_store().await;
    let maker = dot(&store, "Researcher", usd_cap(5.0)).await;
    let one = made(&store, &maker, "Blog Watcher", DotCaps::default()).await;
    let now = Utc::now();
    run_at(&store, &one, now, 1.5, 100).await;
    run_at(&store, &maker, now, 0.5, 10).await;

    let today = dots_core::spend::today(&store, now).await.unwrap();
    let of_maker = row(&today.dots, &maker);
    assert_eq!(of_maker.today, spent(0.5, 10));
    assert_eq!(of_maker.shared_today, Some(spent(2.0, 110)));
    // A made dot that made nothing shares nothing of its own.
    let of_one = row(&today.dots, &one);
    assert_eq!((of_one.today, of_one.shared_today), (spent(1.5, 100), None));
    assert_eq!(today.today, spent(2.0, 110));

    // With the made dot gone the maker's row still shows it.
    store.delete_dot_if_idle(&one.id).await.unwrap();
    let today = dots_core::spend::today(&store, now).await.unwrap();
    assert_eq!(today.dots.len(), 1);
    let of_maker = row(&today.dots, &maker);
    assert_eq!(of_maker.today, spent(0.5, 10));
    assert_eq!(of_maker.shared_today, Some(spent(2.0, 110)));
    assert_eq!(today.today, spent(2.0, 110));
    // The next day there is nothing to share.
    let then = dots_core::spend::today(&store, next_day(now))
        .await
        .unwrap();
    assert_eq!(row(&then.dots, &maker).shared_today, None);
}

#[test]
fn a_dots_spend_without_the_shared_total_reads_as_none() {
    let v = json!({
        "dot_id": "d",
        "dot_name": "Scout",
        "today": {"cost_usd": 1.0, "tokens": 5},
        "caps": {"daily_usd": null, "daily_tokens": null},
    });
    let row: DotSpend = serde_json::from_value(v).unwrap();
    assert_eq!(row.shared_today, None);
}

// The runner: runs wait and are stopped.

fn usage(cost: f64) -> Step {
    Step::Emit(EngineEvent::Usage {
        tokens_in: 100,
        tokens_out: 10,
        cache_read: 0,
        cost_usd: Some(cost),
    })
}

fn finished() -> Step {
    Step::Emit(EngineEvent::Finished {
        summary: "done".into(),
    })
}

struct Env {
    dir: TempDir,
    store: Store,
    runner: Arc<Runner>,
}

async fn env(steps: Vec<Step>) -> Env {
    let (dir, store) = temp_store().await;
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Claude, Arc::new(ScriptedEngine::new(steps)));
    let runner = Runner::new(
        store.clone(),
        new_bus(),
        engines,
        RunnerConfig {
            max_concurrent: 2,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(200),
            plugins_dir: dir.path().join("plugins"),
            quiet_unit: Duration::from_secs(1),
        },
    );
    runner.spawn_dispatcher(CancellationToken::new());
    Env { dir, store, runner }
}

impl Env {
    fn spec(&self, name: &str, daily_usd: Option<f64>) -> DotSpec {
        let mut s = spec(name);
        s.workdir = self.dir.path().to_string_lossy().to_string();
        s.workspace_mode = WorkspaceMode::Folder;
        s.spend_cap = DotCaps {
            daily_usd,
            daily_tokens: None,
        };
        s
    }

    async fn dot(&self, name: &str, daily_usd: Option<f64>) -> Dot {
        self.store
            .create_dot(&self.spec(name, daily_usd), None, "default")
            .await
            .unwrap()
    }

    async fn made(&self, maker: &Dot, name: &str) -> Dot {
        self.store
            .create_made_dot(&self.spec(name, None), None, "default", &made_by(maker))
            .await
            .unwrap()
    }

    /// A finished run of `dot` started now that cost `cost`. The queue is held while the run
    /// is made and ended: a dispatch pass in between would run it.
    async fn spent(&self, dot: &Dot, cost: f64) {
        self.runner.set_held(true).await;
        run_at(&self.store, dot, Utc::now(), cost, 1100).await;
        self.runner.set_held(false).await;
    }

    async fn queue(&self, dot: &Dot) -> String {
        self.runner
            .enqueue(NewRun::new(&dot.id, TriggerKind::Schedule))
            .await
            .unwrap()
            .id
    }

    async fn settle(&self) {
        self.runner.nudge();
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    async fn status(&self, id: &str) -> RunStatus {
        self.store.get_run(id).await.unwrap().status
    }

    /// Why the queued run `id` waits.
    async fn waits_for(&self, id: &str) -> (Option<WaitReason>, Option<String>) {
        let mut runs = vec![self.store.get_run(id).await.unwrap()];
        self.runner.fill_wait_reasons(&mut runs).await.unwrap();
        (runs[0].wait_reason, runs[0].wait_detail.clone())
    }
}

#[tokio::test]
async fn runs_of_the_maker_and_its_made_dots_wait_at_the_shared_cap() {
    let e = env(vec![finished()]).await;
    let maker = e.dot("Researcher", Some(2.0)).await;
    let one = e.made(&maker, "Blog Watcher").await;
    let two = e.made(&maker, "Feed Reader").await;
    e.spent(&one, 2.0).await;

    let of_maker = e.queue(&maker).await;
    let of_one = e.queue(&one).await;
    let of_two = e.queue(&two).await;
    e.settle().await;
    for id in [&of_maker, &of_one, &of_two] {
        assert_eq!(e.status(id).await, RunStatus::Queued);
    }
    // Each says why it waits; a made dot's names the maker.
    assert_eq!(
        e.waits_for(&of_maker).await,
        (
            Some(WaitReason::SpendCap),
            Some("Researcher reached its $2.00 daily cap".into())
        )
    );
    let theirs = "Researcher reached its $2.00 daily cap, shared with the dots it made";
    for id in [&of_one, &of_two] {
        assert_eq!(
            e.waits_for(id).await,
            (Some(WaitReason::SpendCap), Some(theirs.into()))
        );
        let run = e.store.get_run(id).await.unwrap();
        assert_eq!(e.runner.cap_hold(&run).await.unwrap().unwrap().text, theirs);
    }
    // A dot outside the tree runs.
    let other = e.dot("Other", None).await;
    let free = e.queue(&other).await;
    wait_status(&e.store, &free, RunStatus::Succeeded, 20).await;
    assert_eq!(e.status(&of_one).await, RunStatus::Queued);

    // The maker's cap raised: all of them start.
    let mut s = maker.spec.clone();
    s.spend_cap.daily_usd = Some(50.0);
    e.store.update_dot(&maker.id, &s).await.unwrap();
    e.runner.nudge();
    for id in [&of_maker, &of_one, &of_two] {
        wait_status(&e.store, id, RunStatus::Succeeded, 20).await;
    }
}

#[tokio::test]
async fn a_made_dots_own_cap_does_not_hold_its_makers_runs() {
    let e = env(vec![finished()]).await;
    let maker = e.dot("Researcher", None).await;
    let one = e.made(&maker, "Blog Watcher").await;
    let mut s = one.spec.clone();
    s.spend_cap.daily_usd = Some(1.0);
    let one = e.store.update_dot(&one.id, &s).await.unwrap();
    let two = e.made(&maker, "Feed Reader").await;
    e.spent(&one, 1.0).await;

    let of_one = e.queue(&one).await;
    let of_maker = e.queue(&maker).await;
    let of_two = e.queue(&two).await;
    wait_status(&e.store, &of_maker, RunStatus::Succeeded, 20).await;
    wait_status(&e.store, &of_two, RunStatus::Succeeded, 20).await;
    assert_eq!(e.status(&of_one).await, RunStatus::Queued);
    assert_eq!(
        e.waits_for(&of_one).await.1.as_deref(),
        Some("Blog Watcher reached its $1.00 daily cap")
    );
}

#[tokio::test]
async fn a_made_dots_run_is_stopped_when_the_shared_cap_is_crossed() {
    let e = env(vec![usage(0.6), usage(0.6), Step::WaitForCancel]).await;
    let maker = e.dot("Researcher", Some(1.0)).await;
    let one = e.made(&maker, "Blog Watcher").await;
    let id = e.queue(&one).await;
    let run = wait_status(&e.store, &id, RunStatus::Failed, 20).await;
    assert_eq!(
        run.error.as_deref(),
        Some("stopped: Researcher reached its $1.00 daily cap, shared with the dots it made")
    );
    assert!((run.cost_usd.unwrap() - 1.2).abs() < 1e-9);
    // The maker's next run waits, and so does the made dot's.
    let of_maker = e.queue(&maker).await;
    let next = e.queue(&one).await;
    e.settle().await;
    assert_eq!(e.status(&of_maker).await, RunStatus::Queued);
    assert_eq!(e.status(&next).await, RunStatus::Queued);
    assert_eq!(
        e.waits_for(&of_maker).await.1.as_deref(),
        Some("Researcher reached its $1.00 daily cap")
    );
}

#[tokio::test]
async fn a_makers_run_is_stopped_by_what_it_and_its_made_dot_spent() {
    let e = env(vec![usage(0.6), Step::WaitForCancel]).await;
    let maker = e.dot("Researcher", Some(1.0)).await;
    let one = e.made(&maker, "Blog Watcher").await;
    e.spent(&one, 0.5).await;
    let id = e.queue(&maker).await;
    let run = wait_status(&e.store, &id, RunStatus::Failed, 20).await;
    assert_eq!(
        run.error.as_deref(),
        Some("stopped: Researcher reached its $1.00 daily cap")
    );
}

// The runtime: the person's delete and the maker's.

struct T {
    dir: TempDir,
    rt: Runtime,
}

/// As `making_manage_test.rs` has it: no queued run starts, so a run is put in the running
/// state by hand.
async fn start() -> T {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.new_dot_root = dir.path().join("Dots");
    cfg.secrets = Some(Arc::new(MemorySecrets::new()));
    cfg.max_concurrent_runs = 0;
    let rt = Runtime::start(cfg).await.unwrap();
    std::fs::create_dir_all(dir.path().join("work")).unwrap();
    T { dir, rt }
}

/// A maker named Researcher with a $2.00 daily cap, created as the person creates a dot, and
/// a run of it that is running.
async fn capped_maker(t: &T) -> (Dot, Run) {
    let mut s = spec("Researcher");
    s.workdir = t.dir.path().join("work").to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.making.on = true;
    s.spend_cap = usd_cap(2.0);
    let maker = t.rt.create_dot(s, None, None).await.unwrap();
    let mut new = NewRun::new(&maker.id, TriggerKind::Chat);
    new.payload = Some(json!({"message": "watch three blogs", "typed": true}));
    let run = t.rt.store().create_run(&new).await.unwrap();
    assert!(t.rt.store().claim_run(&run.id).await.unwrap());
    (maker, run)
}

/// Makes the dot through the maker's tool; it must be within reach and within the limits.
async fn make(t: &T, run: &Run, name: &str) -> Dot {
    let asked = MakeDot {
        purpose: "Reads the feeds".into(),
        wanted: Wanted {
            name: Some(name.into()),
            instructions: Some("Read the three blogs and list what is new.".into()),
            ..Wanted::default()
        },
        first_job: None,
    };
    match t.rt.makers().make_dot(&run.id, asked).await.unwrap() {
        MakeOutcome::Made { dot, .. } => dot,
        other => panic!("{name} was not made: {other:?}"),
    }
}

#[tokio::test]
async fn a_dot_made_by_a_capped_maker_starts_with_no_cap_of_its_own() {
    let t = start().await;
    let (_maker, run) = capped_maker(&t).await;
    let one = make(&t, &run, "Blog Watcher").await;
    assert_eq!(one.spec.spend_cap, DotCaps::default());
}

#[tokio::test]
async fn the_persons_delete_of_a_made_dot_does_not_lower_the_shared_total() {
    let t = start().await;
    let (maker, run) = capped_maker(&t).await;
    let one = make(&t, &run, "Blog Watcher").await;
    let two = make(&t, &run, "Feed Reader").await;
    let store = t.rt.store();
    let now = Utc::now();
    run_at(store, &one, now, 2.0, 100).await;
    let before = shared(store, &maker, now).await;
    assert_eq!(before, spent(2.0, 100));

    t.rt.delete_dot(&one.id).await.unwrap();

    assert!(matches!(
        store.get_dot(&one.id).await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(shared(store, &maker, now).await, before);
    assert_eq!(
        hit(store, &maker, now).await.as_deref(),
        Some("Researcher reached its $2.00 daily cap")
    );
    assert_eq!(
        hit(store, &two, now).await.as_deref(),
        Some("Researcher reached its $2.00 daily cap, shared with the dots it made")
    );
    let today = t.rt.spend_today().await.unwrap();
    assert_eq!(row(&today.dots, &maker).shared_today, Some(before));
    assert_eq!(today.today, before);
}

#[tokio::test]
async fn a_maker_cannot_clear_its_cap_by_deleting_the_dot_it_made() {
    let t = start().await;
    let (maker, run) = capped_maker(&t).await;
    let one = make(&t, &run, "Blog Watcher").await;
    let store = t.rt.store();
    let now = Utc::now();
    run_at(store, &one, now, 2.0, 100).await;
    let before = shared(store, &maker, now).await;
    assert_eq!(before, spent(2.0, 100));

    t.rt.makers()
        .delete_dot(&run.id, "Blog Watcher")
        .await
        .unwrap();

    assert!(matches!(
        store.get_dot(&one.id).await,
        Err(Error::NotFound(_))
    ));
    assert_eq!(shared(store, &maker, now).await, before);
    assert_eq!(
        hit(store, &maker, now).await.as_deref(),
        Some("Researcher reached its $2.00 daily cap")
    );
    // A dot it makes now is held by the same cap.
    let two = make(&t, &run, "Feed Reader").await;
    assert_eq!(
        hit(store, &two, now).await.as_deref(),
        Some("Researcher reached its $2.00 daily cap, shared with the dots it made")
    );
    assert_eq!(hit(store, &maker, next_day(now)).await, None);
}
