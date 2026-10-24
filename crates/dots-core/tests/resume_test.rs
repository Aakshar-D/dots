mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::{init_repo, spec, temp_store, wait_status};
use dots_core::approvals::{grant_key, ApprovalHub, DecideEffect};
use dots_core::engine::scripted::{ScriptedEngine, Step};
use dots_core::engine::{Engine, EngineEvent};
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::model::{
    ApplyState, Approval, Dot, EngineKind, NewRun, Run, RunStatus, TriggerKind, WorkspaceMode,
};
use dots_core::policy::{Action, Rule};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

struct Env {
    dir: TempDir,
    store: Store,
    bus: Bus,
    hub: Arc<ApprovalHub>,
    runner: Arc<Runner>,
}

async fn env(steps: Vec<Step>) -> Env {
    build_env(steps, None, true).await
}

/// Like `env`, with an engine that performs approved calls itself: a run that is handed such
/// calls follows `resume_steps` instead of `steps`. With `dispatch` false no run starts until
/// the test calls `runner.dispatch_once()`.
async fn applying_env(steps: Vec<Step>, resume_steps: Vec<Step>, dispatch: bool) -> Env {
    build_env(steps, Some(resume_steps), dispatch).await
}

async fn build_env(steps: Vec<Step>, resume_steps: Option<Vec<Step>>, dispatch: bool) -> Env {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let mut engine = ScriptedEngine::with_gate(steps, hub.clone());
    if let Some(resume_steps) = resume_steps {
        engine = engine.applying(resume_steps);
    }
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Claude, Arc::new(engine));
    // A second engine that performs no approved call, for a dot that is moved to it.
    let plain = ScriptedEngine::with_gate(vec![finished()], hub.clone());
    engines.insert(EngineKind::Local, Arc::new(plain));
    // What `Runtime::start` does.
    hub.set_self_apply(
        engines
            .iter()
            .filter(|(_, engine)| engine.applies_approved_calls())
            .map(|(kind, _)| *kind),
    );
    let runner = Runner::new(
        store.clone(),
        bus.clone(),
        engines,
        RunnerConfig {
            max_concurrent: 2,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(200),
            plugins_dir: dir.path().join("plugins"),
        },
    );
    if dispatch {
        runner.spawn_dispatcher(CancellationToken::new());
    }
    Env {
        dir,
        store,
        bus,
        hub,
        runner,
    }
}

async fn parking_dot(e: &Env) -> Dot {
    let mut s = spec("resumer");
    s.workdir = e.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.approval_wait_secs = 0;
    e.store.create_dot(&s, None).await.unwrap()
}

/// What Runtime::decide_approval does (Task 14).
async fn decide(e: &Env, id: &str, approved: bool, note: Option<&str>) -> Option<Run> {
    match e
        .hub
        .decide(id, approved, note.map(String::from))
        .await
        .unwrap()
    {
        DecideEffect::Parked(a) => e.runner.maybe_resume(&a.run_id).await.unwrap(),
        DecideEffect::Live(_) => None,
    }
}

/// Waits until the run is parked and the runner has fully released it (its own post-finish
/// resume check has run), so a decision made by the test is always the one that triggers
/// the resume.
async fn parked(e: &Env, run_id: &str) {
    wait_status(&e.store, run_id, RunStatus::AwaitingApproval, 5).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while e.runner.is_active(run_id) {
        assert!(
            std::time::Instant::now() < deadline,
            "run {run_id} never released"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn ask(cmd: &str) -> Step {
    Step::Ask {
        tool: "Bash".into(),
        input: json!({"command": cmd, "description": "x"}),
    }
}

fn finished() -> Step {
    Step::Emit(EngineEvent::Finished {
        summary: "done".into(),
    })
}

fn message(run: &Run) -> String {
    run.payload.as_ref().unwrap()["message"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test]
async fn approve_after_park_resumes_child_with_grant() {
    let e = env(vec![
        Step::Emit(EngineEvent::SessionStarted {
            session_id: "s1".into(),
        }),
        ask("git push"),
        finished(),
    ])
    .await;
    let dot = parking_dot(&e).await;
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    parked(&e, &parent.id).await;
    let approval = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);

    let child = decide(&e, &approval.id, true, None)
        .await
        .expect("child run");
    assert_eq!(
        e.store.get_run(&parent.id).await.unwrap().status,
        RunStatus::Succeeded
    );
    assert_eq!(child.trigger, TriggerKind::Resume);
    assert_eq!(child.parent_run_id.as_deref(), Some(parent.id.as_str()));
    assert_eq!(child.root_run_id, parent.id);
    assert_eq!(child.session_id.as_deref(), Some("s1"));
    assert!(message(&child).contains(&format!("Approval #{} granted", approval.id)));
    assert_eq!(
        child.payload.as_ref().unwrap()["approvals"],
        json!([approval.id])
    );
    // This engine does not perform approved calls: the approval never gets an apply state.
    assert!(!approval.self_apply);
    assert_eq!(apply_state(&e, &approval.id).await, None);

    wait_status(&e.store, &child.id, RunStatus::Succeeded, 5).await;
    let outputs: Vec<Value> = e
        .store
        .list_events(&child.id, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == "tool_result")
        .map(|ev| ev.data["output"].clone())
        .collect();
    assert_eq!(outputs, vec![json!("allowed")]);
}

#[tokio::test]
async fn deny_with_note_resumes_with_the_note() {
    let e = env(vec![ask("git push"), finished()]).await;
    let dot = parking_dot(&e).await;
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    parked(&e, &parent.id).await;
    let approval = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);
    let child = decide(&e, &approval.id, false, Some("open a PR instead"))
        .await
        .expect("child run");
    assert!(message(&child).contains("Reviewer note: open a PR instead"));
    // No grant was created, so the replayed script parks again.
    wait_status(&e.store, &child.id, RunStatus::AwaitingApproval, 5).await;
}

#[tokio::test]
async fn plain_deny_closes_the_parent_without_a_child() {
    let e = env(vec![ask("git push"), finished()]).await;
    let dot = parking_dot(&e).await;
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    parked(&e, &parent.id).await;
    let approval = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);
    assert!(decide(&e, &approval.id, false, None).await.is_none());
    assert_eq!(
        e.store.get_run(&parent.id).await.unwrap().status,
        RunStatus::Succeeded
    );
    assert_eq!(e.store.list_runs(Some(&dot.id), 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn late_decision_while_running_resumes_after_finish() {
    let e = env(vec![
        ask("git push"),
        Step::Sleep(Duration::from_millis(400)),
        finished(),
    ])
    .await;
    let dot = parking_dot(&e).await;
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let approval = loop {
        let list = e.store.approvals_for_run(&parent.id).await.unwrap();
        if let Some(a) = list.into_iter().find(|a| a.parked) {
            break a;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(
        e.store.get_run(&parent.id).await.unwrap().status,
        RunStatus::Running
    );
    assert!(
        decide(&e, &approval.id, true, None).await.is_none(),
        "run still running"
    );

    let runs = loop {
        let runs = e.store.list_runs(Some(&dot.id), 10).await.unwrap();
        if runs.len() == 2 {
            break runs;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let child = runs.iter().find(|r| r.id != parent.id).unwrap();
    wait_status(&e.store, &child.id, RunStatus::Succeeded, 5).await;
    assert_eq!(
        e.store.get_run(&parent.id).await.unwrap().status,
        RunStatus::Succeeded
    );
}

#[tokio::test]
async fn two_parked_approvals_resume_once_after_both_are_decided() {
    let e = env(vec![ask("git push"), ask("git push --tags"), finished()]).await;
    let dot = parking_dot(&e).await;
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    parked(&e, &parent.id).await;
    let approvals = e.store.approvals_for_run(&parent.id).await.unwrap();
    assert_eq!(approvals.len(), 2);
    assert!(decide(&e, &approvals[0].id, true, None).await.is_none());
    let child = decide(&e, &approvals[1].id, true, None)
        .await
        .expect("child run");
    assert_eq!(
        child.payload.as_ref().unwrap()["approvals"],
        json!([approvals[0].id, approvals[1].id])
    );
    wait_status(&e.store, &child.id, RunStatus::Succeeded, 5).await;
    assert_eq!(e.store.list_runs(Some(&dot.id), 10).await.unwrap().len(), 2);
}

#[tokio::test]
async fn second_maybe_resume_on_same_parent_is_a_noop() {
    let e = env(vec![ask("git push"), finished()]).await;
    let dot = parking_dot(&e).await;
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    parked(&e, &parent.id).await;
    let approval = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);
    assert!(decide(&e, &approval.id, true, None).await.is_some());
    assert!(e.runner.maybe_resume(&parent.id).await.unwrap().is_none());
    assert_eq!(e.store.list_runs(Some(&dot.id), 10).await.unwrap().len(), 2);
}

#[tokio::test]
async fn cancel_before_resume_leaves_parent_cancelled_and_no_child() {
    let e = env(vec![ask("git push"), finished()]).await;
    let dot = parking_dot(&e).await;
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    parked(&e, &parent.id).await;
    let approval = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);
    let effect = e.hub.decide(&approval.id, true, None).await.unwrap();
    assert!(matches!(effect, DecideEffect::Parked(_)));
    assert_eq!(
        e.runner.cancel(&parent.id).await.unwrap().status,
        RunStatus::Cancelled
    );
    assert!(e.runner.maybe_resume(&parent.id).await.unwrap().is_none());
    assert_eq!(
        e.store.get_run(&parent.id).await.unwrap().status,
        RunStatus::Cancelled
    );
    assert_eq!(e.store.list_runs(Some(&dot.id), 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn resume_parked_on_non_awaiting_parent_conflicts_and_changes_nothing() {
    let (_dir, store) = temp_store().await;
    let dot = store.create_dot(&spec("conflicted"), None).await.unwrap();
    let parent = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let a = store
        .create_approval(&parent.id, "Bash", &json!({"command": "x"}))
        .await
        .unwrap();
    store.decide_approval(&a.id, true, None).await.unwrap();
    store.mark_parked(&a.id).await.unwrap();

    let mut child = NewRun::new(&dot.id, TriggerKind::Resume);
    child.parent_run_id = Some(parent.id.clone());
    child.root_run_id = Some(parent.root_run_id.clone());
    // The parent is still `queued`, not `awaiting_approval`.
    let err = store
        .resume_parked(
            &parent.id,
            &[("Bash".into(), "k".into(), None)],
            std::slice::from_ref(&a.id),
            std::slice::from_ref(&a.id),
            Some(&child),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, dots_core::Error::Conflict(_)), "{err:?}");
    assert_eq!(
        store
            .unresolved_parked_decisions(&parent.id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(store.list_runs(Some(&dot.id), 10).await.unwrap().len(), 1);
    assert!(!store
        .take_grant(&parent.root_run_id, "Bash", "k")
        .await
        .unwrap());
    assert_eq!(
        store.get_approval(&a.id).await.unwrap().apply_state,
        None,
        "nothing was queued for Dots to perform"
    );
    assert_eq!(
        store.get_run(&parent.id).await.unwrap().status,
        RunStatus::Queued
    );
}

// ---- Approvals v2: calls that Dots performs itself ----

fn started() -> Step {
    Step::Emit(EngineEvent::SessionStarted {
        session_id: "s1".into(),
    })
}

/// Between two asks: `created_at` has millisecond resolution and orders the calls.
fn gap() -> Step {
    Step::Sleep(Duration::from_millis(5))
}

/// Runs the dot once until it parks. Returns the parent run and its approvals as asked.
async fn parked_parent(e: &Env) -> (Dot, Run, Vec<Approval>) {
    let dot = parking_dot(e).await;
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    parked(e, &parent.id).await;
    let approvals = e.store.approvals_for_run(&parent.id).await.unwrap();
    (dot, parent, approvals)
}

async fn apply_state(e: &Env, id: &str) -> Option<ApplyState> {
    e.store.get_approval(id).await.unwrap().apply_state
}

/// The tool results a run recorded, as (id, output).
async fn results(e: &Env, run_id: &str) -> Vec<(String, String)> {
    e.store
        .list_events(run_id, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == "tool_result")
        .map(|ev| {
            (
                ev.data["id"].as_str().unwrap().to_string(),
                ev.data["output"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

/// Waits until the runner has let go of the run, so everything it does at the end is done.
async fn released(e: &Env, run_id: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while e.runner.is_active(run_id) {
        assert!(
            std::time::Instant::now() < deadline,
            "run {run_id} never released"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn an_approved_call_is_performed_by_the_child_run_and_gets_no_grant() {
    let e = applying_env(
        vec![started(), ask("git push"), finished()],
        vec![Step::ApplyApproved, finished()],
        true,
    )
    .await;
    let (dot, parent, approvals) = parked_parent(&e).await;
    let a = &approvals[0];
    assert!(a.self_apply);
    assert_eq!(a.apply_state, None);

    let child = decide(&e, &a.id, true, None).await.expect("child run");
    assert_eq!(child.trigger, TriggerKind::Resume);
    assert_eq!(child.session_id.as_deref(), Some("s1"));
    // Nothing to review: the engine reports the call once it has a result.
    assert_eq!(
        message(&child),
        "Continue the task, then end with your summary."
    );
    assert_eq!(child.payload.as_ref().unwrap()["approvals"], json!([a.id]));

    wait_status(&e.store, &child.id, RunStatus::Succeeded, 5).await;
    assert_eq!(
        results(&e, &child.id).await,
        vec![(format!("approval:{}", a.id), "performed".to_string())]
    );
    let done = e.store.get_approval(&a.id).await.unwrap();
    assert_eq!(done.apply_state, Some(ApplyState::Applied));
    assert!(done.applied_at.is_some());
    assert!(done.resolved);
    // No grant was written: nothing could let the model repeat the call unasked.
    assert!(!e
        .store
        .take_grant(&parent.root_run_id, "Bash", &grant_key("Bash", &a.input))
        .await
        .unwrap());
    // The child run asked for nothing itself.
    assert!(e
        .store
        .approvals_for_run(&child.id)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(e.store.list_runs(Some(&dot.id), 10).await.unwrap().len(), 2);
}

#[tokio::test]
async fn calls_are_performed_in_the_order_asked_and_a_denial_is_only_reported() {
    let e = applying_env(
        vec![
            ask("one"),
            gap(),
            ask("two"),
            gap(),
            ask("three"),
            finished(),
        ],
        vec![Step::ApplyApproved, finished()],
        true,
    )
    .await;
    let (_dot, _parent, approvals) = parked_parent(&e).await;
    let [one, two, three] = [0, 1, 2].map(|i| approvals[i].id.clone());
    // Decided out of order: the order asked is what counts.
    assert!(decide(&e, &three, true, None).await.is_none());
    assert!(decide(&e, &two, false, Some("not this one"))
        .await
        .is_none());
    let child = decide(&e, &one, true, None).await.expect("child run");
    assert_eq!(
        message(&child),
        format!(
            "Human review of your queued actions:\n\
             - Approval #{two} for Bash was denied. Reviewer note: not this one\n\n\
             Continue the task, then end with your summary."
        )
    );

    wait_status(&e.store, &child.id, RunStatus::Succeeded, 5).await;
    let performed: Vec<String> = results(&e, &child.id)
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        performed,
        vec![format!("approval:{one}"), format!("approval:{three}")]
    );
    assert_eq!(apply_state(&e, &one).await, Some(ApplyState::Applied));
    assert_eq!(apply_state(&e, &two).await, None);
    assert_eq!(apply_state(&e, &three).await, Some(ApplyState::Applied));
}

#[tokio::test]
async fn what_a_resume_run_leaves_open_is_closed_and_announced_when_it_ends() {
    let e = applying_env(
        vec![ask("one"), gap(), ask("two"), finished()],
        vec![
            Step::ClaimApproved,
            Step::Emit(EngineEvent::Failed {
                error: "boom".into(),
            }),
        ],
        true,
    )
    .await;
    let (_dot, _parent, approvals) = parked_parent(&e).await;
    let [one, two] = [0, 1].map(|i| approvals[i].id.clone());
    assert!(decide(&e, &one, true, None).await.is_none());
    let mut events = e.bus.subscribe();
    let child = decide(&e, &two, true, None).await.expect("child run");
    wait_status(&e.store, &child.id, RunStatus::Failed, 5).await;
    released(&e, &child.id).await;

    // Claimed and never given an outcome: nobody knows whether it happened. Never reached:
    // it did not happen.
    assert_eq!(apply_state(&e, &one).await, Some(ApplyState::Unknown));
    assert_eq!(apply_state(&e, &two).await, Some(ApplyState::Skipped));
    let mut updates = Vec::new();
    while let Ok(ev) = events.try_recv() {
        if let RuntimeEvent::ApprovalUpdated { approval } = ev {
            updates.push((approval.id, approval.apply_state));
        }
    }
    assert_eq!(
        updates,
        vec![
            (one.clone(), Some(ApplyState::Applying)),
            (one.clone(), Some(ApplyState::Unknown)),
            (two.clone(), Some(ApplyState::Skipped)),
        ]
    );
    // Nothing performs them later: a new run of the dot is handed no call.
    assert!(e
        .store
        .queued_applies(&approvals[0].run_id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_resume_run_that_parks_again_gives_up_the_calls_it_did_not_perform() {
    let e = applying_env(
        vec![ask("git push"), finished()],
        vec![ask("git tag v1"), finished()],
        true,
    )
    .await;
    let (_dot, _parent, approvals) = parked_parent(&e).await;
    let child = decide(&e, &approvals[0].id, true, None)
        .await
        .expect("child run");
    parked(&e, &child.id).await;
    assert_eq!(
        apply_state(&e, &approvals[0].id).await,
        Some(ApplyState::Skipped)
    );
    // The child's own question is a new approval, waiting for its own decision.
    let asked = e.store.approvals_for_run(&child.id).await.unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].status, dots_core::model::ApprovalStatus::Pending);
    assert_eq!(asked[0].apply_state, None);
}

#[tokio::test]
async fn a_dot_moved_to_an_engine_that_performs_nothing_gets_its_calls_given_up() {
    let e = applying_env(
        vec![ask("git push"), finished()],
        vec![Step::ApplyApproved, finished()],
        true,
    )
    .await;
    let (dot, parent, approvals) = parked_parent(&e).await;
    let a = &approvals[0];
    assert!(a.self_apply);
    // The dot is edited while its run is parked: its runs now go to the other engine.
    let mut moved = dot.spec.clone();
    moved.engine = EngineKind::Local;
    moved.endpoint_url = Some("http://127.0.0.1:9".into());
    e.store.update_dot(&dot.id, &moved).await.unwrap();

    let child = decide(&e, &a.id, true, None).await.expect("child run");
    wait_status(&e.store, &child.id, RunStatus::Succeeded, 5).await;
    released(&e, &child.id).await;
    // The approval was to be performed by Dots, and the engine that now runs the dot does not
    // do that. Nothing performed the call and nothing will: the approval says so, and there
    // is no grant that would let a model repeat it unasked.
    assert_eq!(apply_state(&e, &a.id).await, Some(ApplyState::Skipped));
    assert!(results(&e, &child.id).await.is_empty());
    assert!(!e
        .store
        .take_grant(&parent.root_run_id, "Bash", &grant_key("Bash", &a.input))
        .await
        .unwrap());
}

#[tokio::test]
async fn a_resume_run_cancelled_before_it_starts_performs_nothing() {
    let e = applying_env(
        vec![ask("git push"), finished()],
        vec![Step::ApplyApproved, finished()],
        false,
    )
    .await;
    let dot = parking_dot(&e).await;
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    assert_eq!(e.runner.dispatch_once().await.unwrap(), 1);
    parked(&e, &parent.id).await;
    let a = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);
    let child = decide(&e, &a.id, true, None).await.expect("child run");
    // Nothing dispatches here, so the child waits and its call with it.
    assert_eq!(apply_state(&e, &a.id).await, Some(ApplyState::Queued));

    let mut events = e.bus.subscribe();
    assert_eq!(
        e.runner.cancel(&child.id).await.unwrap().status,
        RunStatus::Cancelled
    );
    assert_eq!(apply_state(&e, &a.id).await, Some(ApplyState::Skipped));
    let mut announced = false;
    while let Ok(ev) = events.try_recv() {
        if let RuntimeEvent::ApprovalUpdated { approval } = ev {
            assert_eq!(approval.apply_state, Some(ApplyState::Skipped));
            announced = true;
        }
    }
    assert!(announced);
    assert_eq!(e.runner.dispatch_once().await.unwrap(), 0);
    assert!(results(&e, &child.id).await.is_empty());
}

// ---- final review: M3 ----

async fn worktree_parking_dot(e: &Env) -> Dot {
    let repo = e.dir.path().join("repo");
    init_repo(&repo);
    let mut s = spec("wt-resumer");
    s.workdir = repo.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Worktree;
    s.approval_wait_secs = 0;
    e.store.create_dot(&s, None).await.unwrap()
}

async fn parked_worktree_run(e: &Env) -> (Run, String) {
    let dot = worktree_parking_dot(e).await;
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    parked(e, &parent.id).await;
    let parent = e.store.get_run(&parent.id).await.unwrap();
    let path = parent.workspace_path.clone().expect("worktree path");
    assert!(
        std::path::Path::new(&path).is_dir(),
        "worktree kept while awaiting"
    );
    (parent, path)
}

#[tokio::test]
async fn plain_deny_close_removes_an_unchanged_worktree() {
    let e = env(vec![ask("git push"), finished()]).await;
    let (parent, path) = parked_worktree_run(&e).await;
    let approval = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);
    assert!(decide(&e, &approval.id, false, None).await.is_none());
    assert_eq!(
        e.store.get_run(&parent.id).await.unwrap().status,
        RunStatus::Succeeded
    );
    assert!(
        !std::path::Path::new(&path).exists(),
        "worktree left behind"
    );
}

/// The resume child takes its folder from its parent's row. A worktree row whose folder is not
/// one of dots' worktrees fails the child before the engine starts in that folder.
#[tokio::test]
async fn a_resumed_run_fails_before_the_engine_starts_when_its_row_names_no_dots_worktree() {
    let e = env(vec![ask("git push"), finished()]).await;
    let (parent, _path) = parked_worktree_run(&e).await;
    let outside = e.dir.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    // As a tampered database would have it: the row's folder is replaced, the rest is kept.
    e.store
        .set_workspace(
            &parent.id,
            &outside.to_string_lossy(),
            parent.branch.as_deref(),
            parent.base_commit.as_deref(),
        )
        .await
        .unwrap();
    let approval = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);
    let child = decide(&e, &approval.id, true, None)
        .await
        .expect("child run");
    assert_eq!(
        child.workspace_path,
        Some(outside.to_string_lossy().to_string())
    );
    let child = wait_status(&e.store, &child.id, RunStatus::Failed, 5).await;
    let error = child.error.unwrap_or_default();
    assert!(error.contains("not a dots worktree"), "{error}");
    // The scripted engine records an event as soon as it starts.
    let events = e.store.list_events(&child.id, 0).await.unwrap();
    assert!(events.is_empty(), "{events:?}");
    assert_eq!(std::fs::read_dir(&outside).unwrap().count(), 0);
}

#[tokio::test]
async fn cancel_of_awaiting_run_removes_an_unchanged_worktree() {
    let e = env(vec![ask("git push"), finished()]).await;
    let (parent, path) = parked_worktree_run(&e).await;
    assert_eq!(
        e.runner.cancel(&parent.id).await.unwrap().status,
        RunStatus::Cancelled
    );
    assert!(
        !std::path::Path::new(&path).exists(),
        "worktree left behind"
    );
}

#[tokio::test]
async fn cancel_of_awaiting_run_publishes_cancelled_after_the_worktree_is_gone() {
    let e = env(vec![ask("git push"), finished()]).await;
    let (parent, path) = parked_worktree_run(&e).await;
    let mut events = e.bus.subscribe();
    let id = parent.id.clone();
    // Looks at the folder the moment the terminal status arrives, as the Inbox does.
    let seen = tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(RuntimeEvent::RunUpdated { run })
                    if run.id == id && run.status == RunStatus::Cancelled =>
                {
                    return std::path::Path::new(&path).exists();
                }
                Ok(_) => {}
                Err(err) => panic!("the event stream ended: {err}"),
            }
        }
    });
    e.runner.cancel(&parent.id).await.unwrap();
    let still_there = tokio::time::timeout(Duration::from_secs(5), seen)
        .await
        .expect("no cancelled status was published")
        .unwrap();
    assert!(
        !still_there,
        "cancelled was published while the unchanged worktree was still on disk"
    );
}

#[tokio::test]
async fn a_granted_context_approval_names_the_tool_the_model_knows() {
    let e = env(vec![
        Step::Ask {
            tool: "Context".into(),
            input: json!({"collection": "company"}),
        },
        finished(),
    ])
    .await;
    let mut s = spec("reader");
    s.workdir = e.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Claude;
    s.approval_wait_secs = 0;
    s.policy.rules.retain(|r| r.tool != "Context");
    s.policy.rules.push(Rule::new("Context", Action::Ask));
    let dot = e.store.create_dot(&s, None).await.unwrap();
    let parent = e
        .runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    parked(&e, &parent.id).await;
    let approval = e
        .store
        .approvals_for_run(&parent.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(approval.tool, "Context");
    let child = decide(&e, &approval.id, true, None)
        .await
        .expect("child run");
    assert!(
        message(&child).contains("you may now use mcp__dots__context with the same input"),
        "{}",
        message(&child)
    );
}
