mod common;

use common::{init_repo, spec, temp_store};
use dots_core::model::{
    Handoff, MemberSpec, NewRun, NewWorkflowRun, ProjectSpec, RunStatus, StepRunChange,
    StepRunStatus, TeamSpec, TriggerKind, WaitingOn, WorkflowRunChange, WorkflowRunStatus,
    WorkflowTrigger,
};
use dots_core::store::Store;
use dots_core::workflow::{Budget, Definition, Step, StepKind, Trigger};
use dots_core::Error;
use serde_json::json;

struct Fixture {
    _dir: tempfile::TempDir,
    store: Store,
    dot: String,
    new: NewWorkflowRun,
}

async fn fixture() -> Fixture {
    let (dir, store) = temp_store().await;
    let dot = store
        .create_dot(&spec("dev"), None, "default")
        .await
        .unwrap()
        .id;
    let team = store
        .create_team(&TeamSpec {
            org_id: None,
            workspace_id: "default".into(),
            name: "T".into(),
            description: String::new(),
            members: vec![MemberSpec {
                dot_id: dot.clone(),
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
                member: dot.clone(),
                job: "Fix it.".into(),
            },
        }],
        budget: Budget {
            max_runs: 4,
            ..Budget::default()
        },
    };
    let workflow = store
        .create_workflow(
            &team.id,
            &dots_core::model::WorkflowSpec {
                name: "Fix".into(),
                definition: definition.clone(),
                enabled: true,
            },
        )
        .await
        .unwrap();
    let new = NewWorkflowRun {
        workflow_id: workflow.id,
        team_id: team.id,
        definition,
        project_id,
        project,
        task: "The tray icon is invisible.".into(),
        trigger: WorkflowTrigger::Manual,
        origin_run_id: None,
        test: false,
    };
    Fixture {
        _dir: dir,
        store,
        dot,
        new,
    }
}

#[tokio::test]
async fn a_workflow_run_keeps_copies_of_its_definition_and_project() {
    let f = fixture().await;
    let wr = f.store.create_workflow_run(&f.new).await.unwrap();
    assert_eq!(wr.status, WorkflowRunStatus::Running);
    assert_eq!(wr.definition, f.new.definition);
    assert_eq!(wr.project, f.new.project);
    assert_eq!(wr.budget.max_runs, 4);
    assert_eq!((wr.spent_runs, wr.spent_tokens, wr.active_ms), (0, 0, 0));
    assert!(wr.steps.is_empty());
    // Deleting the project and the workflow leaves the run whole.
    f.store
        .delete_project(wr.project_id.as_deref().unwrap())
        .await
        .unwrap();
    f.store
        .delete_workflow(wr.workflow_id.as_deref().unwrap())
        .await
        .unwrap();
    let again = f.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(again.project, f.new.project);
    assert_eq!(again.workflow_id, None);
    assert_eq!(
        f.store.workflow_runs_in_flight().await.unwrap(),
        vec![wr.id.clone()]
    );
    assert_eq!(
        f.store
            .list_workflow_runs(Some(&wr.team_id), 10)
            .await
            .unwrap(),
        vec![again]
    );
}

#[tokio::test]
async fn changes_write_only_what_they_name_and_stop_at_the_end() {
    let f = fixture().await;
    let wr = f.store.create_workflow_run(&f.new).await.unwrap();
    let waiting = f
        .store
        .set_workflow_run(
            &wr.id,
            &WorkflowRunChange {
                status: Some(WorkflowRunStatus::Waiting),
                waiting_on: Some(Some(WaitingOn::Check)),
                current_step: Some(Some("a".into())),
                active_ms: Some(1500),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(waiting.waiting_on, Some(WaitingOn::Check));
    assert_eq!(waiting.current_step.as_deref(), Some("a"));
    assert_eq!(waiting.active_ms, 1500);
    assert_eq!(waiting.ended_at, None);
    let done = f
        .store
        .set_workflow_run(
            &wr.id,
            &WorkflowRunChange {
                status: Some(WorkflowRunStatus::Succeeded),
                waiting_on: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(done.ended_at.is_some());
    assert_eq!(done.waiting_on, None);
    assert_eq!(done.active_ms, 1500);
    assert!(f.store.workflow_runs_in_flight().await.unwrap().is_empty());
    assert!(matches!(
        f.store
            .set_workflow_run(
                &wr.id,
                &WorkflowRunChange {
                    status: Some(WorkflowRunStatus::Running),
                    ..Default::default()
                }
            )
            .await,
        Err(Error::Conflict(m)) if m == format!("workflow run {} is already succeeded", wr.id)
    ));
}

#[tokio::test]
async fn step_runs_record_their_hand_off_and_decision() {
    let f = fixture().await;
    let wr = f.store.create_workflow_run(&f.new).await.unwrap();
    let step = f
        .store
        .create_step_run(&wr.id, "a", 1, StepRunStatus::Running)
        .await
        .unwrap();
    assert_eq!((step.attempt, step.status), (1, StepRunStatus::Running));
    let handoff = Handoff {
        summary: "Fixed.".into(),
        branches: vec![],
        outcome: None,
    };
    let done = f
        .store
        .set_step_run(
            &step.id,
            &StepRunChange {
                status: Some(StepRunStatus::Succeeded),
                handoff: Some(Some(handoff.clone())),
                decision: Some(Some(json!({"decision": "approved", "note": null}))),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(done.handoff, Some(handoff));
    assert!(done.ended_at.is_some());
    assert_eq!(done.decision.unwrap()["decision"], "approved");
    assert_eq!(
        f.store.get_workflow_run(&wr.id).await.unwrap().steps,
        vec![f.store.get_step_run(&step.id).await.unwrap()]
    );
}

#[tokio::test]
async fn a_continuing_run_keeps_its_parents_step_and_spend_is_summed() {
    let f = fixture().await;
    let wr = f.store.create_workflow_run(&f.new).await.unwrap();
    let step = f
        .store
        .create_step_run(&wr.id, "a", 1, StepRunStatus::Running)
        .await
        .unwrap();
    let mut first = NewRun::new(&f.dot, TriggerKind::Workflow);
    first.workflow_step_run_id = Some(step.id.clone());
    let first = f.store.create_run(&first).await.unwrap();
    assert_eq!(
        first.workflow_step_run_id.as_deref(),
        Some(step.id.as_str())
    );
    let mut child = NewRun::new(&f.dot, TriggerKind::Resume);
    child.parent_run_id = Some(first.id.clone());
    child.root_run_id = Some(first.root_run_id.clone());
    let child = f.store.create_run(&child).await.unwrap();
    assert_eq!(
        child.workflow_step_run_id.as_deref(),
        Some(step.id.as_str())
    );
    // An ordinary run has none.
    let plain = f
        .store
        .create_run(&NewRun::new(&f.dot, TriggerKind::Manual))
        .await
        .unwrap();
    assert_eq!(plain.workflow_step_run_id, None);

    f.store
        .add_usage(&first.id, 1000, 50, 800, None)
        .await
        .unwrap();
    f.store
        .add_usage(&child.id, 200, 10, 0, None)
        .await
        .unwrap();
    f.store.add_usage(&plain.id, 99, 99, 0, None).await.unwrap();
    let wr = f.store.get_workflow_run(&wr.id).await.unwrap();
    // New input plus output: (1000 - 800 + 50) + (200 + 10); the cache reads apart.
    assert_eq!(
        (wr.spent_runs, wr.spent_tokens, wr.spent_cache_read),
        (2, 460, 800)
    );
    let ids: Vec<_> = f
        .store
        .runs_of_step_run(&step.id)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(ids, [first.id, child.id]);
}

#[tokio::test]
async fn a_run_continuing_a_step_that_has_ended_gets_no_step() {
    let f = fixture().await;
    let wr = f.store.create_workflow_run(&f.new).await.unwrap();
    let step = f
        .store
        .create_step_run(&wr.id, "a", 1, StepRunStatus::Running)
        .await
        .unwrap();
    let mut first = NewRun::new(&f.dot, TriggerKind::Workflow);
    first.workflow_step_run_id = Some(step.id.clone());
    let first = f.store.create_run(&first).await.unwrap();
    assert!(!f.store.in_ended_step_thread(&first).await.unwrap());
    f.store
        .set_step_run(
            &step.id,
            &StepRunChange {
                status: Some(StepRunStatus::Succeeded),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(f.store.in_ended_step_thread(&first).await.unwrap());
    let mut reply = NewRun::new(&f.dot, TriggerKind::Chat);
    reply.parent_run_id = Some(first.id.clone());
    reply.root_run_id = Some(first.root_run_id.clone());
    let reply = f.store.create_run(&reply).await.unwrap();
    assert_eq!(reply.workflow_step_run_id, None);
    assert!(f.store.in_ended_step_thread(&reply).await.unwrap());
    // An ordinary thread is no step's.
    let plain = f
        .store
        .create_run(&NewRun::new(&f.dot, TriggerKind::Chat))
        .await
        .unwrap();
    assert!(!f.store.in_ended_step_thread(&plain).await.unwrap());
}

#[tokio::test]
async fn a_workflow_run_waits_on_the_person_through_a_parked_run_an_approval_or_a_question() {
    let f = fixture().await;
    let wr = f.store.create_workflow_run(&f.new).await.unwrap();
    let step = f
        .store
        .create_step_run(&wr.id, "a", 1, StepRunStatus::Running)
        .await
        .unwrap();
    let mut new = NewRun::new(&f.dot, TriggerKind::Workflow);
    new.workflow_step_run_id = Some(step.id.clone());
    let run = f.store.create_run(&new).await.unwrap();
    let waits = || f.store.workflow_waits_on_person(&wr.id);
    assert!(!waits().await.unwrap());
    // A run parked on an approval.
    f.store
        .finish_run(&run.id, RunStatus::AwaitingApproval, None, None)
        .await
        .unwrap();
    assert!(waits().await.unwrap());
    f.store
        .finish_run(&run.id, RunStatus::Succeeded, Some("done"), None)
        .await
        .unwrap();
    assert!(!waits().await.unwrap());
    // An approval asked and not yet decided.
    let approval = f
        .store
        .create_approval(&run.id, "Bash", &json!({"command": "git push"}))
        .await
        .unwrap();
    assert!(waits().await.unwrap());
    f.store
        .decide_approval(&approval.id, false, None)
        .await
        .unwrap();
    assert!(!waits().await.unwrap());
    // A question neither answered nor dismissed.
    let question = f
        .store
        .add_question(&f.dot, &run.id, "Which icon set?", None)
        .await
        .unwrap()
        .unwrap();
    assert!(waits().await.unwrap());
    f.store.dismiss_question(&question.id).await.unwrap();
    assert!(!waits().await.unwrap());
}
