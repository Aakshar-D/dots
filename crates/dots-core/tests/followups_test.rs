//! The follow-up loop on a clock the test moves by hand (check-ins spec section 6): a due
//! follow-up continues its thread; it waits while the thread is busy and while schedules are
//! paused; a disabled dot's is skipped with a note; one missed while Dots was closed fires once
//! and says how late it is; a cancelled one never fires; a run that cannot be stored is tried
//! after 1, 5 and 30 minutes and then skipped with a note; a deleted dot takes its follow-ups.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::{spec, temp_store};
use dots_core::checkins::local_label;
use dots_core::engine::Engine;
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::followups::{late_words, FollowUps, LATE_AFTER};
use dots_core::model::{
    Dot, EngineKind, FollowUp, FollowUpStatus, MessageKind, NewRun, Run, RunStatus, TriggerKind,
};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use dots_core::util::timestamp;
use dots_core::Error;
use serde_json::json;
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// The fake clock starts here.
const T0: &str = "2030-01-07T09:00:00.000Z";

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(T0)
        .unwrap()
        .with_timezone(&Utc)
}

/// `secs` seconds after `T0`.
fn after(secs: i64) -> DateTime<Utc> {
    t0() + TimeDelta::seconds(secs)
}

/// Runner without a dispatcher: the runs the loop starts stay queued.
fn idle_runner(store: &Store, dir: &TempDir, bus: Bus) -> Arc<Runner> {
    Runner::new(
        store.clone(),
        bus,
        HashMap::<EngineKind, Arc<dyn Engine>>::new(),
        RunnerConfig {
            max_concurrent: 1,
            mcp_url: String::new(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(100),
            plugins_dir: dir.path().join("plugins"),
        },
    )
}

/// The follow-up loop on a fake clock at `T0`, not started yet.
struct Harness {
    dir: TempDir,
    store: Store,
    clock: Arc<FakeClock>,
    bus: Bus,
    runner: Arc<Runner>,
    follow_ups: Arc<FollowUps>,
    shutdown: CancellationToken,
    sleeps: u64,
}

impl Harness {
    async fn new() -> Harness {
        let (dir, store) = temp_store().await;
        let bus = new_bus();
        let clock = FakeClock::new(t0());
        let runner = idle_runner(&store, &dir, bus.clone());
        let follow_ups = FollowUps::new(store.clone(), bus.clone(), clock.clone());
        Harness {
            dir,
            store,
            clock,
            bus,
            runner,
            follow_ups,
            shutdown: CancellationToken::new(),
            sleeps: 0,
        }
    }

    fn start(&self) {
        self.follow_ups
            .spawn(self.runner.clone(), self.shutdown.clone());
    }

    /// Stops the loop and starts a new one on the same database and clock, as a restart of
    /// Dots does.
    fn restart(&mut self) {
        self.shutdown.cancel();
        self.shutdown = CancellationToken::new();
        self.follow_ups = FollowUps::new(self.store.clone(), self.bus.clone(), self.clock.clone());
        self.start();
    }

    /// Waits until the loop sleeps again after its pass, and returns when it means to wake,
    /// in seconds after `T0`.
    async fn settle(&mut self) -> i64 {
        self.sleeps += 1;
        (self.clock.nth_sleep(self.sleeps).await - t0()).num_seconds()
    }

    /// Moves the clock to `after(secs)`, which must reach the loop's wake time, and waits for
    /// the pass that follows.
    async fn at(&mut self, secs: i64) -> i64 {
        self.clock.set(after(secs));
        self.settle().await
    }

    /// A dot and the run that schedules its follow-up: ended `succeeded` with the summary
    /// "Built it.", and with the session `s1` when `session`.
    async fn thread(&self, name: &str, session: bool) -> (Dot, Run) {
        let dot = self.store.create_dot(&spec(name), None).await.unwrap();
        let run = self
            .store
            .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
            .await
            .unwrap();
        if session {
            self.store.set_session(&run.id, "s1").await.unwrap();
        }
        let run = self
            .store
            .finish_run(&run.id, RunStatus::Succeeded, Some("Built it."), None)
            .await
            .unwrap();
        (dot, run)
    }

    /// A follow-up of `run` with the note "check CI", due `secs` after `T0` and scheduled at
    /// `T0`. Written to the store directly: the wake of `FollowUps::schedule` would add a pass
    /// that the test does not count.
    async fn follow_up(&self, run: &Run, secs: i64) -> FollowUp {
        self.store
            .add_follow_up(&run.dot_id, &run.id, after(secs), "check CI", 3, t0())
            .await
            .unwrap()
            .unwrap()
    }

    async fn row(&self, follow_up: &FollowUp) -> FollowUp {
        self.store.get_follow_up(&follow_up.id).await.unwrap()
    }

    /// The run the follow-up fired.
    async fn fired_run(&self, follow_up: &FollowUp) -> Run {
        let row = self.row(follow_up).await;
        assert_eq!(row.status, FollowUpStatus::Fired, "{row:?}");
        self.store
            .get_run(row.fired_run_id.as_deref().unwrap())
            .await
            .unwrap()
    }

    /// Ends `run` with `status` and the session `s1`, as the runner does, and announces it.
    async fn end(&self, run: &Run, status: RunStatus) {
        self.store.set_session(&run.id, "s1").await.unwrap();
        let ended = self
            .store
            .finish_run(&run.id, status, Some("Waited."), None)
            .await
            .unwrap();
        let _ = self.bus.send(RuntimeEvent::RunUpdated { run: ended });
    }

    /// Runs `sql` behind the store's back.
    async fn exec(&self, sql: &str) {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(self.dir.path().join("dots.db"))
            .foreign_keys(true);
        let pool = sqlx::SqlitePool::connect_with(opts).await.unwrap();
        sqlx::query(sql).execute(&pool).await.unwrap();
        pool.close().await;
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

#[tokio::test]
async fn a_due_follow_up_continues_its_thread_with_the_note() {
    let mut h = Harness::new().await;
    let (dot, from) = h.thread("watcher", true).await;
    let follow_up = h.follow_up(&from, 600).await;
    h.start();
    assert_eq!(
        h.settle().await,
        60,
        "nothing is due: the loop sleeps 60 s at most"
    );
    assert_eq!(h.at(600).await, 660);
    let run = h.fired_run(&follow_up).await;
    assert_eq!(run.trigger, TriggerKind::FollowUp);
    assert_eq!(run.status, RunStatus::Queued);
    assert_eq!(run.parent_run_id.as_deref(), Some(from.id.as_str()));
    assert_eq!(run.root_run_id, from.root_run_id);
    assert_eq!(run.session_id.as_deref(), Some("s1"));
    assert_eq!(
        run.payload,
        Some(json!({
            "message": format!("Follow-up you scheduled on {}: check CI", local_label(t0())),
            "follow_up_id": follow_up.id,
        }))
    );
    assert!(h.follow_ups.list(Some(&dot.id)).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_follow_up_whose_thread_ended_without_a_session_starts_afresh() {
    let mut h = Harness::new().await;
    let (_, from) = h.thread("fresh", false).await;
    let follow_up = h.follow_up(&from, 600).await;
    h.start();
    h.settle().await;
    h.at(600).await;
    let run = h.fired_run(&follow_up).await;
    assert_eq!(run.parent_run_id, None);
    assert_eq!(run.root_run_id, run.id);
    assert_eq!(run.session_id, None);
    assert_eq!(
        run.payload.unwrap()["message"],
        format!(
            "Follow-up you scheduled on {}: check CI\nWhat the earlier run reported: Built it.",
            local_label(t0())
        )
    );
}

#[tokio::test]
async fn a_follow_up_waits_while_its_thread_is_busy_and_fires_when_the_run_ends() {
    let mut h = Harness::new().await;
    let (_, from) = h.thread("busy", true).await;
    let follow_up = h.follow_up(&from, 600).await;
    // A chat message continued the thread; its run has not started.
    let mut chat = NewRun::new(&from.dot_id, TriggerKind::Chat);
    chat.parent_run_id = Some(from.id.clone());
    chat.root_run_id = Some(from.root_run_id.clone());
    chat.session_id = Some("s1".into());
    let busy = h.store.create_run(&chat).await.unwrap();
    h.start();
    h.settle().await;
    h.at(600).await;
    assert_eq!(
        h.row(&follow_up).await.status,
        FollowUpStatus::Pending,
        "no second run on a thread"
    );
    h.end(&busy, RunStatus::Succeeded).await;
    h.settle().await;
    let run = h.fired_run(&follow_up).await;
    assert_eq!(run.parent_run_id.as_deref(), Some(busy.id.as_str()));
}

#[tokio::test]
async fn a_follow_up_of_a_disabled_dot_is_skipped_with_a_note() {
    let mut h = Harness::new().await;
    let (dot, from) = h.thread("off", true).await;
    let follow_up = h.follow_up(&from, 600).await;
    h.store.set_dot_enabled(&dot.id, false).await.unwrap();
    let mut events = h.bus.subscribe();
    h.start();
    h.settle().await;
    h.at(600).await;
    let row = h.row(&follow_up).await;
    assert_eq!(row.status, FollowUpStatus::Skipped);
    assert_eq!(row.fired_run_id, None);
    assert_eq!(h.store.list_runs(Some(&dot.id), 10).await.unwrap().len(), 1);
    let notes = h.store.list_messages(Some(&dot.id)).await.unwrap();
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].kind, MessageKind::Note);
    assert_eq!(notes[0].run_id, None);
    assert_eq!(
        notes[0].text,
        format!(
            "A follow-up was skipped because the dot is off. It was due {}: check CI",
            local_label(after(600))
        )
    );
    let (mut posted, mut updated) = (false, false);
    while let Ok(ev) = events.try_recv() {
        match ev {
            RuntimeEvent::MessagePosted { .. } => posted = true,
            RuntimeEvent::FollowUpsUpdated { dot_id } if dot_id == dot.id => updated = true,
            _ => {}
        }
    }
    assert!(posted && updated);
}

#[tokio::test]
async fn a_follow_up_waits_while_schedules_are_paused_and_fires_after() {
    let mut h = Harness::new().await;
    let (_, from) = h.thread("paused", true).await;
    let follow_up = h.follow_up(&from, 600).await;
    h.start();
    h.settle().await;
    h.follow_ups.set_paused(true);
    h.clock.set(after(900));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(h.clock.sleeps(), h.sleeps, "no pass while paused");
    assert_eq!(h.row(&follow_up).await.status, FollowUpStatus::Pending);
    h.follow_ups.set_paused(false);
    h.settle().await;
    let run = h.fired_run(&follow_up).await;
    assert_eq!(
        run.payload.unwrap()["message"],
        format!("Follow-up you scheduled on {}: check CI", local_label(t0())),
        "five minutes late is not late"
    );
}

#[tokio::test]
async fn a_follow_up_missed_while_dots_was_closed_fires_once_with_the_late_words() {
    let mut h = Harness::new().await;
    let (_, from) = h.thread("closed", true).await;
    let follow_up = h.follow_up(&from, 600).await;
    h.clock.set(after(600 + 2 * 3600));
    h.start();
    h.settle().await;
    let run = h.fired_run(&follow_up).await;
    assert_eq!(
        run.payload.unwrap()["message"],
        format!(
            "Follow-up you scheduled on {}: check CI (this is 2 hours later than you asked)",
            local_label(t0())
        )
    );
    h.restart();
    h.settle().await;
    assert_eq!(
        h.store.list_thread(&from.root_run_id).await.unwrap().len(),
        2,
        "it fires once"
    );
}

#[test]
fn a_late_follow_up_says_how_many_hours_late_it_is() {
    let due = t0();
    assert_eq!(late_words(due, due + LATE_AFTER), "");
    assert_eq!(
        late_words(due, due + TimeDelta::minutes(16)),
        " (this is 1 hour later than you asked)"
    );
    assert_eq!(
        late_words(due, due + TimeDelta::minutes(150)),
        " (this is 3 hours later than you asked)"
    );
}

#[tokio::test]
async fn a_cancelled_follow_up_never_fires() {
    let mut h = Harness::new().await;
    let (_, from) = h.thread("cancelled", true).await;
    let follow_up = h.follow_up(&from, 600).await;
    h.start();
    h.settle().await;
    let cancelled = h.follow_ups.cancel(&follow_up.id).await.unwrap();
    assert_eq!(cancelled.status, FollowUpStatus::Cancelled);
    assert_eq!(h.settle().await, 60, "the cancel woke the loop");
    h.at(600).await;
    assert_eq!(
        h.store.list_thread(&from.root_run_id).await.unwrap().len(),
        1
    );
    assert!(matches!(
        h.follow_ups.cancel(&follow_up.id).await,
        Err(Error::Conflict(_))
    ));
}

#[tokio::test]
async fn a_run_that_cannot_be_stored_is_tried_after_1_5_and_30_minutes_then_skipped() {
    let mut h = Harness::new().await;
    let (dot, from) = h.thread("stuck", true).await;
    let follow_up = h.follow_up(&from, 600).await;
    h.exec(
        "CREATE TRIGGER no_runs BEFORE INSERT ON runs BEGIN SELECT RAISE(FAIL, 'disk full'); END",
    )
    .await;
    h.start();
    assert_eq!(h.settle().await, 60);
    assert_eq!(
        h.at(600).await,
        660,
        "the first failure: again after 1 minute"
    );
    let row = h.row(&follow_up).await;
    assert_eq!(
        (row.status, row.attempts, row.retry_at),
        (FollowUpStatus::Pending, 1, Some(timestamp(after(660))))
    );
    assert_eq!(h.at(660).await, 720);
    assert_eq!(
        h.row(&follow_up).await.retry_at,
        Some(timestamp(after(960))),
        "the second: again after 5 minutes"
    );
    assert_eq!(h.at(960).await, 1020);
    assert_eq!(
        h.row(&follow_up).await.retry_at,
        Some(timestamp(after(2760))),
        "the third: again after 30 minutes"
    );
    h.at(2760).await;
    let row = h.row(&follow_up).await;
    assert_eq!((row.status, row.attempts), (FollowUpStatus::Skipped, 4));
    let notes = h.store.list_messages(Some(&dot.id)).await.unwrap();
    assert_eq!(notes.len(), 1);
    let text = &notes[0].text;
    assert!(
        text.starts_with("A follow-up was skipped because its run could not be started ("),
        "{text}"
    );
    assert!(
        text.ends_with(&format!("It was due {}: check CI", local_label(after(600)))),
        "{text}"
    );
}

#[tokio::test]
async fn a_deleted_dot_takes_its_follow_ups_with_it() {
    let mut h = Harness::new().await;
    let (dot, from) = h.thread("gone", true).await;
    let follow_up = h.follow_up(&from, 600).await;
    h.start();
    h.settle().await;
    h.store.delete_dot(&dot.id).await.unwrap();
    assert_eq!(h.at(600).await, 660, "nothing is left to fire");
    assert!(matches!(
        h.store.get_follow_up(&follow_up.id).await,
        Err(Error::NotFound(_))
    ));
    assert!(h.store.list_runs(None, 10).await.unwrap().is_empty());
}
