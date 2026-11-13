//! Scheduled workflows (teams spec section 5.1): the scheduler's pass starts a workflow at the
//! slots of its schedule trigger, never catching up and never retrying, on a fake clock.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, TimeDelta, Utc};
use common::clock::FakeClock;
use common::{init_repo, spec, temp_store};
use dots_core::engine::Engine;
use dots_core::events::{new_bus, Bus};
use dots_core::model::{
    Dot, EngineKind, MemberSpec, ProjectSpec, ScheduleStatus, TeamSpec, WaitingOn, WorkflowRun,
    WorkflowRunChange, WorkflowRunStatus, WorkflowScheduleState, WorkflowSpec, WorkflowTrigger,
};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::schedule::{Rule, Schedule};
use dots_core::scheduler::Scheduler;
use dots_core::store::Store;
use dots_core::teams::Teams;
use dots_core::util::timestamp;
use dots_core::workflow::{Budget, Definition, Step, StepKind, Trigger};
use dots_core::workflows::Workflows;
use dots_core::Error;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

/// The fake clock starts here, years ahead, as in `scheduler_test`.
const T0: &str = "2030-01-07T09:00:00.000Z";

/// The task every scheduled workflow here is started with.
const TASK: &str = "Look for new crash reports and fix the worst one.";

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(T0)
        .unwrap()
        .with_timezone(&Utc)
}

/// `secs` seconds after `T0`.
fn after(secs: i64) -> DateTime<Utc> {
    t0() + TimeDelta::seconds(secs)
}

/// `after(secs)` as the store writes it.
fn ts(secs: i64) -> String {
    timestamp(after(secs))
}

fn schedule(v: Value) -> Schedule {
    serde_json::from_value(v).unwrap()
}

/// Every `minutes`, with slots at `after(offset)` and every `minutes` from there, at most
/// `count` of them.
fn every(minutes: u32, offset: i64, count: Option<u32>) -> Schedule {
    schedule(json!({
        "rule": {"kind": "every", "minutes": minutes, "anchor": ts(offset)},
        "until": count.map(|n| json!({"count": n})),
    }))
}

/// Every day at the local time of `after(secs)`.
fn daily(secs: i64) -> Schedule {
    let time = after(secs)
        .with_timezone(&Local)
        .format("%H:%M")
        .to_string();
    schedule(json!({"rule": {"kind": "daily", "time": time}}))
}

/// Runner without a dispatcher: the scheduler only starts workflow runs, and no workflow loop
/// moves them, so a run stays in flight until the test ends it.
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
            quiet_unit: Duration::from_secs(1),
        },
    )
}

/// A scheduler with workflows, on a fake clock at `T0`, not started yet; a team with the
/// member `dev`, the member `other` and the project "Dots".
struct Harness {
    _dir: TempDir,
    store: Store,
    bus: Bus,
    clock: Arc<FakeClock>,
    runner: Arc<Runner>,
    workflows: Arc<Workflows>,
    sched: Arc<Scheduler>,
    shutdown: CancellationToken,
    sleeps: u64,
    team: String,
    project: String,
    dev: Dot,
    other: Dot,
}

impl Harness {
    async fn new() -> Harness {
        let (dir, store) = temp_store().await;
        let bus = new_bus();
        let clock = FakeClock::new(t0());
        let runner = idle_runner(&store, &dir, bus.clone());
        let workflows = Workflows::new(store.clone(), bus.clone(), runner.clone(), clock.clone());
        let sched = Scheduler::new(store.clone(), runner.clone(), bus.clone(), clock.clone());
        sched.set_workflows(workflows.clone());
        let dev = store
            .create_dot(&spec("dev"), None, "default")
            .await
            .unwrap();
        let other = store
            .create_dot(&spec("other"), None, "default")
            .await
            .unwrap();
        let team = store
            .create_team(&TeamSpec {
                org_id: None,
                workspace_id: "default".into(),
                name: "Sage management".into(),
                description: String::new(),
                members: vec![member(&dev, "dev agent"), member(&other, "reviewer")],
            })
            .await
            .unwrap()
            .id;
        let repo = dir.path().join("project");
        init_repo(&repo);
        let project = store
            .add_project(
                &team,
                &ProjectSpec {
                    name: "Dots".into(),
                    repo_path: repo.to_string_lossy().to_string(),
                    default_branch: None,
                    remote: "origin".into(),
                },
            )
            .await
            .unwrap()
            .id;
        Harness {
            _dir: dir,
            store,
            bus,
            clock,
            runner,
            workflows,
            sched,
            shutdown: CancellationToken::new(),
            sleeps: 0,
            team,
            project,
            dev,
            other,
        }
    }

    /// A workflow started by `schedule` on the project named `project`, whose one step is
    /// `member`'s.
    fn spec(&self, name: &str, schedule: Schedule, project: &str, member: &Dot) -> WorkflowSpec {
        WorkflowSpec {
            name: name.into(),
            definition: Definition {
                trigger: Trigger::Schedule {
                    schedule,
                    task: TASK.into(),
                    project: project.into(),
                },
                steps: vec![Step {
                    id: "fix".into(),
                    name: "Fix it".into(),
                    kind: StepKind::Agent {
                        member: member.id.clone(),
                        job: "Fix the bug.".into(),
                        can_send_back_to: vec![],
                        outcomes: vec![],
                        delegate: None,
                    },
                }],
                budget: Budget::default(),
            },
            enabled: true,
        }
    }

    /// Saves a scheduled workflow of the dev on "Dots" and returns its id.
    async fn workflow(&self, name: &str, schedule: Schedule) -> String {
        let spec = self.spec(name, schedule, "Dots", &self.dev);
        self.store
            .create_workflow(&self.team, &spec)
            .await
            .unwrap()
            .id
    }

    fn start(&self) {
        self.sched.spawn(self.shutdown.clone());
    }

    /// Stops the scheduler (Dots is closed).
    fn stop(&mut self) {
        self.shutdown.cancel();
        self.shutdown = CancellationToken::new();
    }

    /// Starts a new scheduler on the same database and clock, as Dots starting again does.
    fn restart(&mut self) {
        self.stop();
        self.sched = Scheduler::new(
            self.store.clone(),
            self.runner.clone(),
            self.bus.clone(),
            self.clock.clone(),
        );
        self.sched.set_workflows(self.workflows.clone());
        self.start();
    }

    /// Waits until the loop sleeps again after its pass, and returns when it means to wake, in
    /// seconds after `T0`.
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

    /// Wakes the loop, as a save does, and waits for the pass that follows.
    async fn reload(&mut self) -> i64 {
        self.sched.reload();
        self.settle().await
    }

    async fn row(&self, workflow: &str) -> Option<WorkflowScheduleState> {
        self.store
            .get_workflow_schedule_state(workflow)
            .await
            .unwrap()
    }

    /// The workflow's runs, newest first.
    async fn runs(&self, workflow: &str) -> Vec<WorkflowRun> {
        self.store
            .list_workflow_runs(Some(&self.team), 50)
            .await
            .unwrap()
            .into_iter()
            .filter(|wr| wr.workflow_id.as_deref() == Some(workflow))
            .collect()
    }

    /// Moves the workflow run to `status`, as its loop would.
    async fn set_status(&self, run: &str, status: WorkflowRunStatus) {
        let waiting_on = (status == WorkflowRunStatus::Waiting).then_some(WaitingOn::Check);
        self.store
            .set_workflow_run(
                run,
                &WorkflowRunChange {
                    status: Some(status),
                    waiting_on: Some(waiting_on),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}

fn member(dot: &Dot, role: &str) -> MemberSpec {
    MemberSpec {
        dot_id: dot.id.clone(),
        role: role.into(),
    }
}

#[tokio::test]
async fn a_daily_workflow_starts_at_its_slot_with_the_trigger_s_task_and_project() {
    let mut h = Harness::new().await;
    // The trigger names the project in other case: projects are found without case.
    let spec = h.spec("Crash reports", daily(600), "dots", &h.dev);
    let id = h.store.create_workflow(&h.team, &spec).await.unwrap().id;
    h.start();
    assert_eq!(h.settle().await, 60, "the slot is 10 minutes away");
    let row = h.row(&id).await.expect("the pass made the row");
    assert_eq!(row.status, ScheduleStatus::Active);
    assert_eq!(row.last_slot_at, T0, "made at the pass's time");
    assert_eq!(row.fired_count, 0);
    assert!(h.runs(&id).await.is_empty());

    h.at(300).await;
    assert!(h.runs(&id).await.is_empty(), "not before its slot");
    h.at(600).await;
    let runs = h.runs(&id).await;
    assert_eq!(runs.len(), 1);
    let wr = &runs[0];
    assert_eq!(wr.trigger, WorkflowTrigger::Schedule);
    assert_eq!(wr.task, TASK);
    assert_eq!(wr.project_id.as_deref(), Some(h.project.as_str()));
    assert_eq!(wr.project.name, "Dots");
    assert_eq!(wr.origin_run_id, None);
    assert!(!wr.test);
    assert_eq!(wr.status, WorkflowRunStatus::Running);
    let row = h.row(&id).await.unwrap();
    assert_eq!(row.last_slot_at, ts(600));
    assert_eq!(row.fired_count, 1);

    // Its run ends; nothing more starts until the next day's slot.
    h.set_status(&wr.id, WorkflowRunStatus::Succeeded).await;
    h.at(660).await;
    h.at(86_400).await;
    assert_eq!(h.runs(&id).await.len(), 1);
    h.at(86_400 + 600).await;
    assert_eq!(h.runs(&id).await.len(), 2);
}

#[tokio::test]
async fn a_slot_while_a_run_is_in_flight_is_skipped_and_the_next_one_starts_after_it_ended() {
    let mut h = Harness::new().await;
    let id = h.workflow("Every 10", every(10, 0, None)).await;
    h.start();
    h.settle().await;
    h.at(600).await;
    let first = h.runs(&id).await;
    assert_eq!(first.len(), 1);

    // Running at the next slot: skipped, not counted, and not started later.
    h.at(1200).await;
    assert_eq!(h.runs(&id).await.len(), 1, "no second run while one runs");
    let row = h.row(&id).await.unwrap();
    assert_eq!(row.last_slot_at, ts(1200), "the slot is used up");
    assert_eq!(row.fired_count, 1);
    h.at(1260).await;
    assert_eq!(h.runs(&id).await.len(), 1);

    // Waiting on the person's check is in flight too.
    h.set_status(&first[0].id, WorkflowRunStatus::Waiting).await;
    h.at(1800).await;
    assert_eq!(h.runs(&id).await.len(), 1, "no second run while one waits");

    // Once it ended, the next slot starts a run.
    h.set_status(&first[0].id, WorkflowRunStatus::Succeeded)
        .await;
    h.at(2400).await;
    let runs = h.runs(&id).await;
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[0].status, WorkflowRunStatus::Running);
    assert_eq!(h.row(&id).await.unwrap().fired_count, 2);
}

/// "Never catch up": after a gap, the first pass marks the newest missed slot handled and
/// starts nothing. A slot less than `ON_TIME` (60 s) old at that pass is on time, not missed.
#[tokio::test]
async fn slots_missed_while_dots_was_closed_never_start_late() {
    let mut h = Harness::new().await;
    let id = h.workflow("Every 10", every(10, 0, None)).await;
    h.start();
    h.settle().await;

    // Closed over the slots at 600, 1200 and 1800; back at 2000.
    h.stop();
    h.clock.set(after(2000));
    h.restart();
    assert_eq!(h.settle().await, 2060);
    assert!(h.runs(&id).await.is_empty(), "no missed slot runs");
    let row = h.row(&id).await.unwrap();
    assert_eq!(
        row.last_slot_at,
        ts(1800),
        "the newest missed slot is handled"
    );
    assert_eq!(row.fired_count, 0);
    h.at(2060).await;
    assert!(h.runs(&id).await.is_empty(), "nor later");

    // The next slot starts on time.
    h.at(2400).await;
    let runs = h.runs(&id).await;
    assert_eq!(runs.len(), 1);
    h.set_status(&runs[0].id, WorkflowRunStatus::Succeeded)
        .await;

    // Back 30 s after a slot: that slot is on time, and starts.
    h.stop();
    h.clock.set(after(3030));
    h.restart();
    h.settle().await;
    assert_eq!(h.runs(&id).await.len(), 2);
    assert_eq!(h.row(&id).await.unwrap().last_slot_at, ts(3000));
}

#[tokio::test]
async fn a_turned_off_workflow_and_one_with_a_problem_do_not_start() {
    let mut h = Harness::new().await;
    let on = h.workflow("On", every(10, 0, None)).await;
    let mut off = h.spec("Off", every(10, 0, None), "Dots", &h.dev);
    off.enabled = false;
    let off = h.store.create_workflow(&h.team, &off).await.unwrap();
    let broken = h.spec("Broken", every(10, 0, None), "Dots", &h.other);
    let broken = h.store.create_workflow(&h.team, &broken).await.unwrap().id;
    // `other` leaves the team: the workflow naming it has a problem.
    h.store
        .update_team(
            &h.team,
            &TeamSpec {
                org_id: None,
                workspace_id: "default".into(),
                name: "Sage management".into(),
                description: String::new(),
                members: vec![member(&h.dev, "dev agent")],
            },
        )
        .await
        .unwrap();
    assert!(h
        .store
        .get_workflow(&broken)
        .await
        .unwrap()
        .problem
        .is_some());

    h.start();
    h.settle().await;
    h.at(600).await;
    assert_eq!(h.runs(&on).await.len(), 1, "the pass ran");
    assert!(h.runs(&off.id).await.is_empty());
    assert!(h.runs(&broken).await.is_empty());
    assert!(h.row(&off.id).await.is_none());
    assert!(h.row(&broken).await.is_none());

    // Turned on through `Teams`: its change wakes the loop (before its wake time, 660), and
    // the pass makes the row at that time.
    h.clock.set(after(630));
    let teams = Teams::new(h.store.clone(), h.bus.clone());
    let mut spec = off.spec.clone();
    spec.enabled = true;
    teams.update_workflow(&off.id, &spec).await.unwrap();
    h.settle().await;
    assert_eq!(h.row(&off.id).await.unwrap().last_slot_at, ts(630));
    h.at(1200).await;
    assert_eq!(h.runs(&off.id).await.len(), 1);

    // Turned off again: its row goes, and a later turn on starts afresh.
    spec.enabled = false;
    h.store.update_workflow(&off.id, &spec).await.unwrap();
    h.reload().await;
    assert!(h.row(&off.id).await.is_none());
}

#[tokio::test]
async fn pausing_schedules_pauses_scheduled_workflows() {
    let mut h = Harness::new().await;
    let id = h.workflow("Every 10", every(10, 0, None)).await;
    h.start();
    h.settle().await;

    h.sched.set_paused(true);
    // The slot at 600 comes and is 30 s old at the resume.
    h.clock.set(after(630));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(h.clock.sleeps(), h.sleeps, "no pass while paused");
    h.sched.set_paused(false);
    h.settle().await;
    assert!(
        h.runs(&id).await.is_empty(),
        "the pause's slots do not start"
    );
    assert_eq!(
        h.row(&id).await.unwrap().last_slot_at,
        ts(630),
        "the resume time"
    );

    h.at(1200).await;
    assert_eq!(h.runs(&id).await.len(), 1);
}

#[tokio::test]
async fn a_changed_rule_resets_the_row() {
    let mut h = Harness::new().await;
    let id = h.workflow("Twice", every(10, 0, Some(2))).await;
    h.start();
    h.settle().await;
    h.at(600).await;
    let runs = h.runs(&id).await;
    assert_eq!(runs.len(), 1);
    h.set_status(&runs[0].id, WorkflowRunStatus::Succeeded)
        .await;
    let before = h.row(&id).await.unwrap();
    assert_eq!(before.fired_count, 1);

    // Every 15 minutes from 300 s: slots at 1200, 2100 and on.
    let spec = h.spec("Twice", every(15, 300, Some(2)), "Dots", &h.dev);
    h.store.update_workflow(&id, &spec).await.unwrap();
    // Before the loop's wake time (660), so only the reload brings a pass.
    h.clock.set(after(650));
    h.reload().await;
    let row = h.row(&id).await.unwrap();
    assert_ne!(row.rule_key, before.rule_key);
    assert_eq!(row.last_slot_at, ts(650), "reset at the pass's time");
    assert_eq!(row.fired_count, 0, "the count starts again");

    h.at(1200).await;
    assert_eq!(h.runs(&id).await.len(), 2);

    // Saved again unchanged: the row stays.
    h.store.update_workflow(&id, &spec).await.unwrap();
    h.reload().await;
    assert_eq!(h.row(&id).await.unwrap().fired_count, 1);
}

#[tokio::test]
async fn until_count_2_stops_after_two_runs() {
    let mut h = Harness::new().await;
    let id = h.workflow("Twice", every(10, 0, Some(2))).await;
    h.start();
    h.settle().await;
    for slot in [600, 1200] {
        h.at(slot).await;
        let runs = h.runs(&id).await;
        h.set_status(&runs[0].id, WorkflowRunStatus::Succeeded)
            .await;
    }
    let row = h.row(&id).await.unwrap();
    assert_eq!(row.fired_count, 2);
    assert_eq!(row.status, ScheduleStatus::Completed);

    h.at(1800).await;
    h.at(2400).await;
    assert_eq!(h.runs(&id).await.len(), 2);
    assert_eq!(h.row(&id).await.unwrap().last_slot_at, ts(1200));
}

#[tokio::test]
async fn a_start_that_is_refused_uses_up_its_slot() {
    let mut h = Harness::new().await;
    let id = h.workflow("Every 10", every(10, 0, None)).await;
    h.store.delete_project(&h.project).await.unwrap();
    h.start();
    h.settle().await;
    h.at(600).await;
    assert!(h.runs(&id).await.is_empty());
    let row = h.row(&id).await.unwrap();
    assert_eq!(row.last_slot_at, ts(600), "the slot is used up");
    assert_eq!(row.fired_count, 0);

    // Not retried when the project is back; the next slot starts.
    assert!(h.store.get_team(&h.team).await.unwrap().projects.is_empty());
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    h.store
        .add_project(
            &h.team,
            &ProjectSpec {
                name: "Dots".into(),
                repo_path: dir.path().to_string_lossy().to_string(),
                default_branch: None,
                remote: "origin".into(),
            },
        )
        .await
        .unwrap();
    h.at(660).await;
    assert!(h.runs(&id).await.is_empty());
    h.at(1200).await;
    assert_eq!(h.runs(&id).await.len(), 1);
}

#[tokio::test]
async fn an_every_schedule_saved_gets_its_anchor_and_keeps_it() {
    let h = Harness::new().await;
    let teams = Teams::new(h.store.clone(), h.bus.clone());
    let anchor = |w: &dots_core::model::Workflow| match &w.spec.definition.trigger {
        Trigger::Schedule {
            schedule:
                Schedule {
                    rule: Rule::Every { anchor, .. },
                    ..
                },
            ..
        } => *anchor,
        other => panic!("not an every schedule: {other:?}"),
    };
    let unanchored =
        |minutes: u32| schedule(json!({"rule": {"kind": "every", "minutes": minutes}}));

    let before = Utc::now() - TimeDelta::seconds(1);
    let spec = h.spec("Every 30", unanchored(30), "Dots", &h.dev);
    let made = teams.create_workflow(&h.team, &spec).await.unwrap();
    let first = anchor(&made).expect("saving set the anchor");
    assert!(first >= before && first <= Utc::now());
    assert_eq!(
        anchor(&h.store.get_workflow(&made.id).await.unwrap()),
        Some(first)
    );

    // Saved again without one (the UI never sends it), with the same minutes: kept.
    let kept = teams.update_workflow(&made.id, &spec).await.unwrap();
    assert_eq!(anchor(&kept), Some(first));

    // New minutes: it starts again.
    tokio::time::sleep(Duration::from_millis(5)).await;
    let spec = h.spec("Every 30", unanchored(45), "Dots", &h.dev);
    let moved = teams.update_workflow(&made.id, &spec).await.unwrap();
    assert!(anchor(&moved).unwrap() > first);

    // A schedule that would never run is refused, as a dot's is.
    let once = schedule(json!({"rule": {"kind": "once", "at": "2020-01-01T09:00"}}));
    let spec = h.spec("Long ago", once, "Dots", &h.dev);
    match teams.create_workflow(&h.team, &spec).await {
        Err(Error::Invalid(m)) => {
            assert_eq!(m, "the trigger's schedule: this schedule would never run")
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test]
async fn deleting_a_workflow_deletes_its_schedule_row() {
    let mut h = Harness::new().await;
    let id = h.workflow("Every 10", every(10, 0, None)).await;
    h.start();
    h.settle().await;
    assert!(h.row(&id).await.is_some());
    h.stop();
    h.store.delete_workflow(&id).await.unwrap();
    assert!(h.row(&id).await.is_none());
    assert!(h
        .store
        .list_workflow_schedule_states()
        .await
        .unwrap()
        .is_empty());
}
