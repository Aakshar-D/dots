//! Sending a scheduled message (send later spec sections 4.2 to 4.5), on a `FakeClock`: the
//! target read at send time (a reply after the topic's end, a wait while the topic is busy or
//! Claude's limit is reached, a new topic when the old one cannot go on), the races with Edit,
//! Cancel and Send now, recovery after a crash, and the sender loop. The queue is held before
//! any run is created, so no dispatcher claims one mid-test.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use common::clock::FakeClock;
use common::{init_repo, spec};
use dots_core::events::RuntimeEvent;
use dots_core::model::{
    Dot, DotSpec, MemberSpec, NewRun, NewWorkflowRun, ProjectSpec, Run, RunStatus,
    ScheduledMessage, ScheduledStatus, ScheduledWait, StepRunChange, StepRunStatus, TeamSpec,
    TriggerKind, WorkflowSpec, WorkflowTrigger, WorkspaceMode,
};
use dots_core::send_later::{SendTarget, ALREADY_SENT, BEING_SENT, SEND_EVERY, TOO_FAR, TOO_SOON};
use dots_core::util::timestamp;
use dots_core::workflow::{Budget, Definition, Step, StepKind, Trigger};
use dots_core::{Config, Error, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio::sync::broadcast::Receiver;

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-04T09:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn at(offset: TimeDelta) -> String {
    timestamp(t0() + offset)
}

fn hour() -> TimeDelta {
    TimeDelta::hours(1)
}

fn config(data: &Path, clock: &Arc<FakeClock>) -> Config {
    let mut cfg = Config::new(data.to_path_buf());
    cfg.port = 0;
    cfg.clock = clock.clone();
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg
}

/// A runtime on `data` at `now`, with its queue held.
async fn runtime_at(data: &Path, now: DateTime<Utc>) -> (Runtime, Arc<FakeClock>) {
    let clock = FakeClock::new(now);
    let rt = Runtime::start(config(data, &clock)).await.unwrap();
    rt.hold_runs(true).await;
    (rt, clock)
}

async fn start(dir: &TempDir) -> (Runtime, Arc<FakeClock>) {
    runtime_at(&dir.path().join("data"), t0()).await
}

fn folder_spec(name: &str, dir: &TempDir) -> DotSpec {
    let mut s: DotSpec = spec(name);
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s
}

async fn dot(rt: &Runtime, dir: &TempDir) -> Dot {
    rt.create_dot(folder_spec("courier", dir), None, None)
        .await
        .unwrap()
}

/// Ends the queued run `run`: succeeded with `session`, or failed before the dot started when
/// `session` is `None`.
async fn end(rt: &Runtime, run: &Run, session: Option<&str>) -> Run {
    let store = rt.store();
    assert!(store.claim_run(&run.id).await.unwrap());
    match session {
        Some(s) => {
            store.mark_delivered(&run.id, Utc::now()).await.unwrap();
            store.set_session(&run.id, s).await.unwrap();
            store
                .finish_run(&run.id, RunStatus::Succeeded, Some("done"), None)
                .await
                .unwrap()
        }
        None => store
            .finish_run(
                &run.id,
                RunStatus::Failed,
                None,
                Some("claude CLI not found"),
            )
            .await
            .unwrap(),
    }
}

/// A topic: the person's first message, answered with session `s1`.
async fn topic(rt: &Runtime, dot: &Dot) -> Run {
    let p = rt.chat(&dot.id, "Hello", None).await.unwrap();
    end(rt, &p, Some("s1")).await
}

async fn schedule(rt: &Runtime, dot: &Dot, root: Option<&Run>, due: TimeDelta) -> ScheduledMessage {
    rt.schedule_message(&dot.id, "Ship it", root.map(|r| r.id.as_str()), &at(due))
        .await
        .unwrap()
}

async fn row(rt: &Runtime, id: &str) -> ScheduledMessage {
    rt.store().get_scheduled_message(id).await.unwrap()
}

async fn sent_run(rt: &Runtime, message: &ScheduledMessage) -> Run {
    let message = row(rt, &message.id).await;
    assert_eq!(message.status, ScheduledStatus::Sent, "{message:?}");
    rt.get_run(message.run_id.as_deref().unwrap())
        .await
        .unwrap()
}

fn note(run: &Run) -> Value {
    run.payload.as_ref().unwrap()["send_later"].clone()
}

async fn runs_of(rt: &Runtime, dot: &Dot) -> usize {
    rt.store()
        .list_runs(Some(&dot.id), 100)
        .await
        .unwrap()
        .len()
}

/// The `ScheduledMessageUpdated` rows sent so far.
fn updates(rx: &mut Receiver<RuntimeEvent>) -> Vec<ScheduledMessage> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let RuntimeEvent::ScheduledMessageUpdated { message } = ev {
            out.push(message);
        }
    }
    out
}

fn conflict(e: Error) -> String {
    match e {
        Error::Conflict(m) => m,
        other => panic!("expected Conflict, got {other:?}"),
    }
}

fn invalid(e: Error) -> String {
    match e {
        Error::Invalid(m) => m,
        other => panic!("expected Invalid, got {other:?}"),
    }
}

#[tokio::test]
async fn scheduling_validates_and_lists() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, _clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let other = rt
        .create_dot(folder_spec("other", &dir), None, None)
        .await
        .unwrap();
    let foreign = rt.chat(&other.id, "Hi", None).await.unwrap();
    let mut rx = rt.subscribe();

    let refused = |due: TimeDelta, text: &'static str| {
        let rt = &rt;
        let dot = &dot;
        async move {
            let e = rt
                .schedule_message(&dot.id, text, None, &at(due))
                .await
                .unwrap_err();
            invalid(e)
        }
    };
    assert_eq!(refused(TimeDelta::seconds(30), "Ship it").await, TOO_SOON);
    assert_eq!(refused(-hour(), "Ship it").await, TOO_SOON);
    assert_eq!(refused(TimeDelta::days(366), "Ship it").await, TOO_FAR);
    assert_eq!(refused(hour(), "  ").await, "message must not be empty");
    let e = rt
        .schedule_message(&dot.id, "Ship it", Some(&foreign.id), &at(hour()))
        .await
        .unwrap_err();
    assert!(invalid(e).contains("belongs to a different dot"));
    assert!(updates(&mut rx).is_empty());

    let message = schedule(&rt, &dot, None, hour()).await;
    assert_eq!(message.status, ScheduledStatus::Scheduled);
    assert_eq!(message.due_at, at(hour()));
    assert_eq!(
        rt.scheduled_messages(&dot.id).await.unwrap(),
        std::slice::from_ref(&message)
    );
    assert_eq!(updates(&mut rx), [message]);
    rt.shutdown();
}

#[tokio::test]
async fn a_due_message_replies_to_its_topic() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let p = topic(&rt, &dot).await;
    let message = schedule(&rt, &dot, Some(&p), hour()).await;

    // Not due yet: nothing goes.
    assert!(rt.send_due().await.unwrap().is_empty());
    assert_eq!(
        row(&rt, &message.id).await.status,
        ScheduledStatus::Scheduled
    );

    clock.set(t0() + hour());
    rt.send_due().await.unwrap();
    let r = sent_run(&rt, &message).await;
    assert_eq!(r.trigger, TriggerKind::Chat);
    assert_eq!(r.parent_run_id.as_deref(), Some(p.id.as_str()));
    assert_eq!(r.session_id.as_deref(), Some("s1"));
    assert_eq!(r.payload.as_ref().unwrap()["message"], "Ship it");
    assert_eq!(r.payload.as_ref().unwrap()["typed"], true);
    assert_eq!(
        note(&r),
        json!({ "id": message.id, "due_at": at(hour()), "late": false, "new_topic": false })
    );
    let sent = row(&rt, &message.id).await;
    assert_eq!(sent.sent_at, Some(at(hour())));
    assert!(rt.scheduled_messages(&dot.id).await.unwrap().is_empty());
    rt.shutdown();
}

#[tokio::test]
async fn a_message_with_no_topic_starts_one() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let message = schedule(&rt, &dot, None, hour()).await;
    clock.set(t0() + hour());
    rt.send_due().await.unwrap();
    let r = sent_run(&rt, &message).await;
    assert_eq!(r.parent_run_id, None);
    assert_eq!(note(&r)["new_topic"], false);
    rt.shutdown();
}

#[tokio::test]
async fn a_busy_topic_waits_then_replies_to_its_newest_run() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let p = topic(&rt, &dot).await;
    let message = schedule(&rt, &dot, Some(&p), hour()).await;
    // The person typed into the topic meanwhile; the dot has not answered yet.
    let q = rt.chat(&dot.id, "And then?", Some(&p.id)).await.unwrap();
    let mut rx = rt.subscribe();

    clock.set(t0() + hour());
    rt.send_due().await.unwrap();
    let waiting = row(&rt, &message.id).await;
    assert_eq!(waiting.status, ScheduledStatus::Scheduled);
    assert_eq!(waiting.waiting, Some(ScheduledWait::Busy));
    assert_eq!(runs_of(&rt, &dot).await, 2);
    assert_eq!(updates(&mut rx), [waiting]);

    // The same reason again: nothing is sent, not even an event.
    rt.send_due().await.unwrap();
    assert!(updates(&mut rx).is_empty());

    end(&rt, &q, Some("s1")).await;
    clock.set(t0() + hour() + TimeDelta::seconds(30));
    rt.send_due().await.unwrap();
    let r = sent_run(&rt, &message).await;
    assert_eq!(r.parent_run_id.as_deref(), Some(q.id.as_str()));
    assert_eq!(row(&rt, &message.id).await.waiting, None);
    rt.shutdown();
}

#[tokio::test]
async fn an_approval_in_the_topic_waits_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let p = topic(&rt, &dot).await;
    let message = schedule(&rt, &dot, Some(&p), hour()).await;
    let q = rt.chat(&dot.id, "Deploy", Some(&p.id)).await.unwrap();
    rt.store()
        .set_status(&q.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();

    clock.set(t0() + hour());
    rt.send_due().await.unwrap();
    assert_eq!(
        row(&rt, &message.id).await.waiting,
        Some(ScheduledWait::Approval)
    );
    assert_eq!(runs_of(&rt, &dot).await, 2);

    // Approved, and the run ended: the next pass replies to it.
    let store = rt.store();
    store.mark_delivered(&q.id, Utc::now()).await.unwrap();
    store.set_session(&q.id, "s1").await.unwrap();
    store
        .finish_run(&q.id, RunStatus::Succeeded, Some("deployed"), None)
        .await
        .unwrap();
    clock.set(t0() + hour() + SEND_EVERY);
    rt.send_due().await.unwrap();
    let r = sent_run(&rt, &message).await;
    assert_eq!(r.parent_run_id.as_deref(), Some(q.id.as_str()));
    assert_eq!(row(&rt, &message.id).await.waiting, None);
    rt.shutdown();
}

#[tokio::test]
async fn a_lost_race_in_the_chat_path_waits() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let p = topic(&rt, &dot).await;
    let message = schedule(&rt, &dot, Some(&p), hour()).await;
    // The thread moved on after the target was read: the target is stale.
    let q = rt.chat(&dot.id, "And then?", Some(&p.id)).await.unwrap();
    end(&rt, &q, Some("s1")).await;

    clock.set(t0() + hour());
    let out = rt
        .send_scheduled_to(
            row(&rt, &message.id).await,
            SendTarget::Reply {
                parent_run_id: p.id.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(out.status, ScheduledStatus::Scheduled);
    assert_eq!(out.waiting, Some(ScheduledWait::Busy));
    assert_eq!(runs_of(&rt, &dot).await, 2);

    // The next pass reads the thread again and replies to its end.
    rt.send_due().await.unwrap();
    let r = sent_run(&rt, &message).await;
    assert_eq!(r.parent_run_id.as_deref(), Some(q.id.as_str()));
    rt.shutdown();
}

#[tokio::test]
async fn a_topic_that_cannot_continue_gets_a_new_topic() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    // Its only run failed before the dot had a session.
    let p = rt.chat(&dot.id, "Hello", None).await.unwrap();
    let p = end(&rt, &p, None).await;
    let no_session = schedule(&rt, &dot, Some(&p), hour()).await;
    // Its root is gone.
    let gone = rt
        .store()
        .insert_scheduled_message(&dot.id, Some("01NOSUCHRUN"), "Ship it", &at(hour()), t0())
        .await
        .unwrap();
    // Its root is a run of another dot (only the store could write that).
    let other = rt
        .create_dot(folder_spec("other", &dir), None, None)
        .await
        .unwrap();
    let foreign = topic(&rt, &other).await;
    let elsewhere = rt
        .store()
        .insert_scheduled_message(&dot.id, Some(&foreign.id), "Ship it", &at(hour()), t0())
        .await
        .unwrap();
    // Its topic is a workflow step's thread, and the step has ended.
    let step_run = ended_step_thread(&rt, &dot, &dir).await;
    let step = schedule(&rt, &dot, Some(&step_run), hour()).await;

    clock.set(t0() + hour());
    rt.send_due().await.unwrap();
    for message in [&no_session, &gone, &elsewhere, &step] {
        let r = sent_run(&rt, message).await;
        assert_eq!(r.dot_id, dot.id);
        assert_eq!(r.parent_run_id, None);
        assert_eq!(r.root_run_id, r.id);
        assert_eq!(note(&r)["new_topic"], true);
    }
    rt.shutdown();
}

/// A run of `dot` for a workflow step, answered with a session, whose step has ended since.
async fn ended_step_thread(rt: &Runtime, dot: &Dot, dir: &TempDir) -> Run {
    let store = rt.store();
    let team = store
        .create_team(&TeamSpec {
            org_id: None,
            workspace_id: "default".into(),
            name: "T".into(),
            description: String::new(),
            members: vec![MemberSpec {
                dot_id: dot.id.clone(),
                role: "dev".into(),
            }],
        })
        .await
        .unwrap();
    let repo = dir.path().join("repo");
    init_repo(&repo);
    let project = ProjectSpec {
        name: "Dots".into(),
        repo_path: repo.to_string_lossy().to_string(),
        default_branch: None,
        remote: "origin".into(),
    };
    let project_id = store.add_project(&team.id, &project).await.unwrap().id;
    let definition = Definition {
        trigger: Trigger::Manual,
        steps: vec![Step {
            id: "a".into(),
            name: "Work".into(),
            kind: StepKind::Agent {
                member: dot.id.clone(),
                job: "Fix it.".into(),
                can_send_back_to: vec![],
                outcomes: vec![],
                delegate: None,
            },
        }],
        budget: Budget::default(),
    };
    let workflow = store
        .create_workflow(
            &team.id,
            &WorkflowSpec {
                name: "Fix".into(),
                definition: definition.clone(),
                enabled: true,
            },
        )
        .await
        .unwrap();
    let wr = store
        .create_workflow_run(&NewWorkflowRun {
            workflow_id: workflow.id,
            team_id: team.id,
            definition,
            project_id,
            project,
            task: "The tray icon is invisible.".into(),
            trigger: WorkflowTrigger::Manual,
            origin_run_id: None,
            test: false,
        })
        .await
        .unwrap();
    let step = store
        .create_step_run(&wr.id, "a", 1, StepRunStatus::Running)
        .await
        .unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Workflow);
    new.workflow_step_run_id = Some(step.id.clone());
    let run = store.create_run(&new).await.unwrap();
    let run = end(rt, &run, Some("s1")).await;
    store
        .set_step_run(
            &step.id,
            &StepRunChange {
                status: Some(StepRunStatus::Succeeded),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(store.in_ended_step_thread(&run).await.unwrap());
    run
}

/// Claude's reading as a run's `rate_limit_event` records it: rejected until `until`. (How a
/// CLI line becomes this reading is the limits tests' part.)
async fn reject_until(rt: &Runtime, until: DateTime<Utc>) {
    rt.limits()
        .record_reset("rejected", Vec::new(), Some(&timestamp(until)))
        .await
        .unwrap();
    assert!(rt.limits_get().await.unwrap().rejected_until.is_some());
}

#[tokio::test]
async fn the_usage_limit_waits() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    reject_until(&rt, t0() + TimeDelta::hours(2)).await;

    let dot = dot(&rt, &dir).await;
    let message = schedule(&rt, &dot, None, hour()).await;
    clock.set(t0() + hour());
    rt.send_due().await.unwrap();
    let waiting = row(&rt, &message.id).await;
    assert_eq!(waiting.waiting, Some(ScheduledWait::UsageLimit));
    assert_eq!(runs_of(&rt, &dot).await, 0);

    clock.set(t0() + TimeDelta::hours(2) + TimeDelta::minutes(1));
    rt.send_due().await.unwrap();
    let r = sent_run(&rt, &message).await;
    assert_eq!(note(&r)["late"], true);
    rt.shutdown();
}

/// The chat path refuses at Claude's limit after the target was read: the message waits for
/// the reset. Any other refusal at the limit is not a wait: it fails with its own reason.
#[tokio::test]
async fn a_limit_reached_after_the_target_was_read_waits() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let message = schedule(&rt, &dot, None, hour()).await;
    clock.set(t0() + hour());
    reject_until(&rt, t0() + TimeDelta::hours(2)).await;

    let out = rt
        .send_scheduled_to(
            row(&rt, &message.id).await,
            SendTarget::New { fallback: false },
        )
        .await
        .unwrap();
    assert_eq!(out.status, ScheduledStatus::Scheduled);
    assert_eq!(out.waiting, Some(ScheduledWait::UsageLimit));

    // A message the chat path refuses for its text (only the store could write it).
    let blank = rt
        .store()
        .insert_scheduled_message(&dot.id, None, " ", &at(TimeDelta::zero()), t0())
        .await
        .unwrap();
    let out = rt
        .send_scheduled_to(blank, SendTarget::New { fallback: false })
        .await
        .unwrap();
    assert_eq!(out.status, ScheduledStatus::Failed);
    assert_eq!(out.error.as_deref(), Some("message must not be empty"));
    assert_eq!(runs_of(&rt, &dot).await, 0);
    rt.shutdown();
}

#[tokio::test]
async fn a_refused_dot_fails_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    // A dot of an org this PC is no longer joined to.
    let dot = rt
        .store()
        .create_dot(&folder_spec("triage", &dir), Some("acme"), "default")
        .await
        .unwrap();
    let message = rt
        .store()
        .insert_scheduled_message(&dot.id, None, "Ship it", &at(hour()), t0())
        .await
        .unwrap();

    clock.set(t0() + hour());
    rt.send_due().await.unwrap();
    let failed = row(&rt, &message.id).await;
    assert_eq!(failed.status, ScheduledStatus::Failed);
    assert_eq!(
        failed.error.as_deref(),
        Some("dot 'triage': its org 'acme' was removed")
    );
    assert_eq!(runs_of(&rt, &dot).await, 0);
    rt.shutdown();
}

#[tokio::test]
async fn late_is_more_than_a_minute() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let on_time = schedule(&rt, &dot, None, hour()).await;
    clock.set(t0() + hour() + TimeDelta::seconds(30));
    rt.send_due().await.unwrap();
    assert_eq!(note(&sent_run(&rt, &on_time).await)["late"], false);

    let late = schedule(&rt, &dot, None, TimeDelta::hours(2)).await;
    clock.set(t0() + TimeDelta::hours(2) + TimeDelta::minutes(2));
    rt.send_due().await.unwrap();
    assert_eq!(note(&sent_run(&rt, &late).await)["late"], true);
    rt.shutdown();
}

#[tokio::test]
async fn two_due_for_one_topic_go_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let p = topic(&rt, &dot).await;
    let a = schedule(&rt, &dot, Some(&p), hour()).await;
    let b = schedule(&rt, &dot, Some(&p), hour() + TimeDelta::minutes(1)).await;

    // Dots was closed over both.
    clock.set(t0() + TimeDelta::hours(3));
    rt.send_due().await.unwrap();
    let ra = sent_run(&rt, &a).await;
    assert_eq!(ra.parent_run_id.as_deref(), Some(p.id.as_str()));
    assert_eq!(note(&ra)["late"], true);
    assert_eq!(row(&rt, &b.id).await.waiting, Some(ScheduledWait::Busy));

    end(&rt, &ra, Some("s1")).await;
    rt.send_due().await.unwrap();
    let rb = sent_run(&rt, &b).await;
    assert_eq!(rb.parent_run_id.as_deref(), Some(ra.id.as_str()));
    assert_eq!(runs_of(&rt, &dot).await, 3);
    rt.shutdown();
}

#[tokio::test]
async fn edit_and_cancel_lose_to_a_claim() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let message = schedule(&rt, &dot, None, hour()).await;
    clock.set(t0() + hour());
    rt.store()
        .claim_scheduled_message(&message.id, clock_now(&clock))
        .await
        .unwrap()
        .unwrap();

    let later = at(TimeDelta::hours(3));
    let e = rt
        .edit_scheduled_message(&message.id, "Other", &later)
        .await
        .unwrap_err();
    assert_eq!(conflict(e), BEING_SENT);
    let e = rt.cancel_scheduled_message(&message.id).await.unwrap_err();
    assert_eq!(conflict(e), BEING_SENT);

    let run = rt.chat(&dot.id, "Ship it", None).await.unwrap();
    rt.store()
        .mark_scheduled_sent(&message.id, &run.id, clock_now(&clock))
        .await
        .unwrap();
    let e = rt
        .edit_scheduled_message(&message.id, "Other", &later)
        .await
        .unwrap_err();
    assert_eq!(conflict(e), ALREADY_SENT);
    let e = rt.cancel_scheduled_message(&message.id).await.unwrap_err();
    assert_eq!(conflict(e), ALREADY_SENT);
    assert_eq!(row(&rt, &message.id).await.text, "Ship it");
    rt.shutdown();
}

fn clock_now(clock: &FakeClock) -> DateTime<Utc> {
    use dots_core::scheduler::Clock;
    clock.now()
}

#[tokio::test]
async fn edit_moves_the_time_and_cancel_keeps_the_row() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let message = schedule(&rt, &dot, None, hour()).await;
    let edited = rt
        .edit_scheduled_message(&message.id, "Ship it today", &at(TimeDelta::hours(2)))
        .await
        .unwrap();
    assert_eq!(edited.text, "Ship it today");
    assert_eq!(edited.due_at, at(TimeDelta::hours(2)));
    let e = rt
        .edit_scheduled_message(&message.id, "x", &at(TimeDelta::seconds(10)))
        .await
        .unwrap_err();
    assert_eq!(invalid(e), TOO_SOON);

    // Not due at its old time any more.
    clock.set(t0() + hour());
    rt.send_due().await.unwrap();
    assert_eq!(runs_of(&rt, &dot).await, 0);

    let cancelled = rt.cancel_scheduled_message(&message.id).await.unwrap();
    assert_eq!(cancelled.status, ScheduledStatus::Cancelled);
    assert_eq!(
        row(&rt, &message.id).await.status,
        ScheduledStatus::Cancelled
    );
    let e = rt.cancel_scheduled_message(&message.id).await.unwrap_err();
    assert_eq!(conflict(e), "This message was cancelled.");
    rt.shutdown();
}

#[tokio::test]
async fn the_sender_skips_a_cancelled_message() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let message = schedule(&rt, &dot, None, hour()).await;
    rt.cancel_scheduled_message(&message.id).await.unwrap();
    clock.set(t0() + TimeDelta::hours(2));
    assert!(rt.send_due().await.unwrap().is_empty());
    // A stale read of the row loses its claim too.
    let out = rt
        .send_scheduled_to(message.clone(), SendTarget::New { fallback: false })
        .await
        .unwrap();
    assert_eq!(out.status, ScheduledStatus::Cancelled);
    assert_eq!(runs_of(&rt, &dot).await, 0);
    rt.shutdown();
}

#[tokio::test]
async fn send_now_sends_at_once_and_is_not_late() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, _clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let message = schedule(&rt, &dot, None, TimeDelta::days(1)).await;

    let sent = rt.send_scheduled_now(&message.id).await.unwrap();
    assert_eq!(sent.status, ScheduledStatus::Sent);
    assert_eq!(sent.due_at, timestamp(t0()));
    assert_eq!(note(&sent_run(&rt, &message).await)["late"], false);
    let e = rt.send_scheduled_now(&message.id).await.unwrap_err();
    assert_eq!(conflict(e), ALREADY_SENT);

    // A failed message of a dot that may run again.
    let failed = rt
        .store()
        .insert_scheduled_message(&dot.id, None, "Again", &at(TimeDelta::zero()), t0())
        .await
        .unwrap();
    rt.store()
        .mark_scheduled_failed(&failed.id, "org removed", t0())
        .await
        .unwrap();
    let sent = rt.send_scheduled_now(&failed.id).await.unwrap();
    assert_eq!(sent.status, ScheduledStatus::Sent);
    assert_eq!(sent.error, None);
    assert_eq!(runs_of(&rt, &dot).await, 2);
    rt.shutdown();
}

/// Two Send nows at once: one sends, the other is refused (send later spec section 4.4).
#[tokio::test]
async fn two_send_nows_at_once_send_once() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, _clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let message = schedule(&rt, &dot, None, TimeDelta::days(1)).await;

    let (a, b) = tokio::join!(
        rt.send_scheduled_now(&message.id),
        rt.send_scheduled_now(&message.id)
    );
    let (sent, refused): (Vec<_>, Vec<_>) = [a, b].into_iter().partition(|r| r.is_ok());
    assert_eq!(sent.len(), 1, "{sent:?} {refused:?}");
    assert_eq!(sent[0].as_ref().unwrap().status, ScheduledStatus::Sent);
    let text = conflict(refused.into_iter().next().unwrap().unwrap_err());
    assert!(text == BEING_SENT || text == ALREADY_SENT, "{text}");
    assert_eq!(runs_of(&rt, &dot).await, 1);
    rt.shutdown();
}

/// A send whose run was stored but whose row was not marked sent (the write failed) is settled
/// by the next pass, not left "Sending…" until the next start.
#[tokio::test]
async fn a_pass_settles_a_send_whose_last_write_failed() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let dot = dot(&rt, &dir).await;
    let message = schedule(&rt, &dot, None, hour()).await;
    clock.set(t0() + hour());
    rt.store()
        .claim_scheduled_message(&message.id, t0() + hour())
        .await
        .unwrap()
        .unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
    new.payload = Some(json!({
        "message": "Ship it", "typed": true,
        "send_later": { "id": message.id, "due_at": message.due_at, "late": false, "new_topic": false },
    }));
    let run = rt.store().create_run(&new).await.unwrap();
    let mut rx = rt.subscribe();

    let changed = rt.send_due().await.unwrap();
    assert_eq!(changed.len(), 1);
    let sent = row(&rt, &message.id).await;
    assert_eq!(sent.status, ScheduledStatus::Sent);
    assert_eq!(sent.run_id.as_deref(), Some(run.id.as_str()));
    assert_eq!(updates(&mut rx), [sent]);
    assert_eq!(runs_of(&rt, &dot).await, 1);
    rt.shutdown();
}

#[tokio::test]
async fn a_crash_while_sending_never_sends_twice() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let due = t0() + hour();

    // Claimed, and the crash came before its run was stored.
    let (rt, clock) = runtime_at(&data, t0()).await;
    let dot = dot(&rt, &dir).await;
    let lost = schedule(&rt, &dot, None, hour()).await;
    clock.set(due);
    rt.store()
        .claim_scheduled_message(&lost.id, due)
        .await
        .unwrap()
        .unwrap();
    // Claimed, and the crash came after its run was stored.
    let stored = rt
        .store()
        .insert_scheduled_message(&dot.id, None, "Stored", &at(TimeDelta::zero()), t0())
        .await
        .unwrap();
    rt.store()
        .claim_scheduled_message(&stored.id, due)
        .await
        .unwrap()
        .unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
    new.payload = Some(json!({
        "message": "Stored", "typed": true,
        "send_later": { "id": stored.id, "due_at": stored.due_at, "late": true, "new_topic": false },
    }));
    let run = rt.store().create_run(&new).await.unwrap();
    end(&rt, &run, Some("s1")).await;
    rt.shutdown();
    drop(rt);

    let (rt, _clock) = runtime_at(&data, due).await;
    let lost = row(&rt, &lost.id).await;
    assert_eq!(lost.status, ScheduledStatus::Scheduled);
    let stored = row(&rt, &stored.id).await;
    assert_eq!(stored.status, ScheduledStatus::Sent);
    assert_eq!(stored.run_id.as_deref(), Some(run.id.as_str()));

    rt.send_due().await.unwrap();
    assert_eq!(row(&rt, &lost.id).await.status, ScheduledStatus::Sent);
    rt.send_due().await.unwrap();
    assert_eq!(runs_of(&rt, &dot).await, 2);
    rt.shutdown();
}

#[tokio::test]
async fn the_sender_sends_on_the_clock() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let rt = Arc::new(rt);
    let dot = dot(&rt, &dir).await;
    let mut rx = rt.subscribe();
    let sender = rt.spawn_send_later();
    let message = schedule(&rt, &dot, None, TimeDelta::minutes(5)).await;

    // Nothing goes before its time, whatever the loop does meanwhile.
    clock.set(t0() + TimeDelta::minutes(4));
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        row(&rt, &message.id).await.status,
        ScheduledStatus::Scheduled
    );

    clock.set(t0() + TimeDelta::minutes(5));
    let sent = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(RuntimeEvent::ScheduledMessageUpdated { message: m }) = rx.recv().await {
                if m.id == message.id && m.status == ScheduledStatus::Sent {
                    return m;
                }
            }
        }
    })
    .await
    .expect("the sender sent the message at its time");
    assert_eq!(sent.sent_at, Some(at(TimeDelta::minutes(5))));
    assert_eq!(runs_of(&rt, &dot).await, 1);
    rt.shutdown();
    sender.await.unwrap();
}

/// The next `ScheduledMessageUpdated` of the row `id` that is sent, waiting at most `wait`.
async fn sent_within(
    rx: &mut Receiver<RuntimeEvent>,
    id: &str,
    wait: Duration,
) -> Option<ScheduledMessage> {
    tokio::time::timeout(wait, async {
        loop {
            match rx.recv().await {
                Ok(RuntimeEvent::ScheduledMessageUpdated { message })
                    if message.id == id && message.status == ScheduledStatus::Sent =>
                {
                    return message;
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("the event bus closed: {e}"),
            }
        }
    })
    .await
    .ok()
}

/// A due message that waits on a busy topic is sent by the loop itself once the topic is free:
/// its 30-second cap tries again, with no schedule or edit to wake it. The clock is moved 30
/// seconds at a time until the row is sent, as the loop may not be asleep yet at the first move.
#[tokio::test]
async fn the_sender_retries_a_waiting_message_by_itself() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, clock) = start(&dir).await;
    let rt = Arc::new(rt);
    let dot = dot(&rt, &dir).await;
    let p = topic(&rt, &dot).await;
    let message = schedule(&rt, &dot, Some(&p), TimeDelta::minutes(5)).await;
    let q = rt.chat(&dot.id, "And then?", Some(&p.id)).await.unwrap();
    clock.set(t0() + TimeDelta::minutes(5));
    let mut rx = rt.subscribe();
    let sender = rt.spawn_send_later();

    // The first pass finds the topic busy.
    let waiting = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(RuntimeEvent::ScheduledMessageUpdated { message: m }) = rx.recv().await {
                if m.id == message.id && m.waiting == Some(ScheduledWait::Busy) {
                    return m;
                }
            }
        }
    })
    .await
    .expect("the first pass found the topic busy");
    assert_eq!(waiting.status, ScheduledStatus::Scheduled);

    end(&rt, &q, Some("s1")).await;
    let mut sent = None;
    for _ in 0..50 {
        clock.set(clock_now(&clock) + SEND_EVERY);
        sent = sent_within(&mut rx, &message.id, Duration::from_millis(200)).await;
        if sent.is_some() {
            break;
        }
    }
    let sent = sent.expect("the sender sent the waiting message once the topic was free");
    let r = rt.get_run(sent.run_id.as_deref().unwrap()).await.unwrap();
    assert_eq!(r.parent_run_id.as_deref(), Some(q.id.as_str()));
    assert_eq!(runs_of(&rt, &dot).await, 3);
    rt.shutdown();
    sender.await.unwrap();
}

/// Dots was closed over the due time: after a restart the loop's first pass sends the message,
/// late, and only once; the one a crash left with its run stored is not sent again.
#[tokio::test]
async fn the_sender_sends_once_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("data");
    let (rt, _clock) = runtime_at(&data, t0()).await;
    let dot = dot(&rt, &dir).await;
    let p = topic(&rt, &dot).await;
    let message = schedule(&rt, &dot, Some(&p), hour()).await;
    let stored = rt
        .store()
        .insert_scheduled_message(&dot.id, None, "Stored", &at(TimeDelta::zero()), t0())
        .await
        .unwrap();
    rt.store()
        .claim_scheduled_message(&stored.id, t0())
        .await
        .unwrap()
        .unwrap();
    let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
    new.payload = Some(json!({
        "message": "Stored", "typed": true,
        "send_later": { "id": stored.id, "due_at": stored.due_at, "late": false, "new_topic": false },
    }));
    let stored_run = rt.store().create_run(&new).await.unwrap();
    end(&rt, &stored_run, Some("s2")).await;
    rt.shutdown();
    drop(rt);

    let (rt, _clock) = runtime_at(&data, t0() + TimeDelta::hours(3)).await;
    let rt = Arc::new(rt);
    let mut rx = rt.subscribe();
    let sender = rt.spawn_send_later();
    // The first pass, with no move of the clock.
    let sent = sent_within(&mut rx, &message.id, Duration::from_secs(10))
        .await
        .expect("the first pass sent the message that came due while Dots was closed");
    let r = rt.get_run(sent.run_id.as_deref().unwrap()).await.unwrap();
    assert_eq!(r.parent_run_id.as_deref(), Some(p.id.as_str()));
    assert_eq!(note(&r)["late"], true);
    let stored = row(&rt, &stored.id).await;
    assert_eq!(stored.status, ScheduledStatus::Sent);
    assert_eq!(stored.run_id.as_deref(), Some(stored_run.id.as_str()));
    // The topic's first run, the one the crash left, and this one.
    assert_eq!(runs_of(&rt, &dot).await, 3);
    rt.shutdown();
    sender.await.unwrap();
}
