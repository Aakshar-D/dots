mod common;

use common::{init_repo, spec, temp_store};
use dots_core::model::{
    Handoff, LastRun, MemberSpec, NewRun, NewWorkflowRun, ProjectSpec, Run, RunStatus, StepDotRun,
    StepRunChange, StepRunStatus, TeamSpec, TriggerKind, WaitingOn, WaitingWorkflowRun,
    WorkflowRun, WorkflowRunChange, WorkflowRunFilter, WorkflowRunStatus, WorkflowTrigger,
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
                can_send_back_to: vec![],
                outcomes: vec![],
                delegate: None,
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
        members: vec![],
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
async fn step_runs_record_their_outcome_and_notice() {
    let f = fixture().await;
    let wr = f.store.create_workflow_run(&f.new).await.unwrap();
    let step = f
        .store
        .create_step_run(&wr.id, "a", 1, StepRunStatus::Running)
        .await
        .unwrap();
    assert_eq!(
        (step.outcome.as_deref(), step.notice.as_deref()),
        (None, None)
    );
    let reported = f
        .store
        .set_step_outcome(&step.id, Some("fixed"))
        .await
        .unwrap();
    assert_eq!(reported.outcome.as_deref(), Some("fixed"));
    assert_eq!(reported.status, StepRunStatus::Running);
    let noticed = f
        .store
        .set_step_run(
            &step.id,
            &StepRunChange {
                notice: Some(Some("Started again from the start.".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        noticed.notice.as_deref(),
        Some("Started again from the start.")
    );
    // A change that does not name them leaves them as they are.
    let done = f
        .store
        .set_step_run(
            &step.id,
            &StepRunChange {
                status: Some(StepRunStatus::Succeeded),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(done.outcome.as_deref(), Some("fixed"));
    assert_eq!(
        done.notice.as_deref(),
        Some("Started again from the start.")
    );
    // Both clear.
    let cleared = f.store.set_step_outcome(&step.id, None).await.unwrap();
    assert_eq!(cleared.outcome, None);
    let cleared = f
        .store
        .set_step_run(
            &step.id,
            &StepRunChange {
                notice: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(cleared.notice, None);
    assert!(matches!(
        f.store.set_step_outcome("nope", Some("fixed")).await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn a_step_run_records_the_send_back_it_asked_for() {
    let f = fixture().await;
    let wr = f.store.create_workflow_run(&f.new).await.unwrap();
    let step = f
        .store
        .create_step_run(&wr.id, "a", 1, StepRunStatus::Running)
        .await
        .unwrap();
    assert_eq!(step.send_back, None);
    let asked = json!({"to": "a", "note": "Use the light icon."});
    let recorded = f
        .store
        .set_step_send_back(&step.id, Some(&asked))
        .await
        .unwrap();
    assert_eq!(recorded.send_back, Some(asked.clone()));
    // The workflow run reads it with its steps, and a change that does not name it keeps it.
    let done = f
        .store
        .set_step_run(
            &step.id,
            &StepRunChange {
                status: Some(StepRunStatus::Succeeded),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(done.send_back, Some(asked.clone()));
    let read = f.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(read.steps[0].send_back, Some(asked));
    let cleared = f.store.set_step_send_back(&step.id, None).await.unwrap();
    assert_eq!(cleared.send_back, None);
    assert!(matches!(
        f.store.set_step_send_back("nope", None).await,
        Err(Error::NotFound(_))
    ));
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

#[tokio::test]
async fn a_member_is_added_with_its_run_and_reported_with_the_run_that_continues_the_step() {
    let f = fixture().await;
    let wr = f.store.create_workflow_run(&f.new).await.unwrap();
    let step = f
        .store
        .create_step_run(&wr.id, "a", 1, StepRunStatus::Running)
        .await
        .unwrap();
    let mut lead = NewRun::new(&f.dot, TriggerKind::Workflow);
    lead.workflow_step_run_id = Some(step.id.clone());
    let lead = f.store.create_run(&lead).await.unwrap();
    assert_eq!(f.store.step_member(&lead.root_run_id).await.unwrap(), None);
    // A member's first run is its thread's root, and works for the step.
    let mut new = NewRun::new(&f.dot, TriggerKind::Workflow);
    new.workflow_step_run_id = Some(step.id.clone());
    let one = f.store.add_step_member(&new, "Part one.").await.unwrap();
    let two = f.store.add_step_member(&new, "Part two.").await.unwrap();
    assert_eq!(one.root_run_id, one.id);
    assert_eq!(one.workflow_step_run_id.as_deref(), Some(step.id.as_str()));
    let members = f.store.step_members(&step.id).await.unwrap();
    let rows: Vec<_> = members
        .iter()
        .map(|m| {
            (
                m.run_id.as_str(),
                m.dot_id.as_str(),
                m.task.as_str(),
                m.reported,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (one.id.as_str(), f.dot.as_str(), "Part one.", false),
            (two.id.as_str(), f.dot.as_str(), "Part two.", false),
        ]
    );
    assert_eq!(
        f.store.step_member(&one.id).await.unwrap().as_ref(),
        Some(&members[0])
    );
    // Both count against the budget, with the lead's run.
    assert_eq!(
        f.store.get_workflow_run(&wr.id).await.unwrap().spent_runs,
        3
    );
    // The run that tells the lead is stored with the members marked reported, once.
    let mut told = NewRun::new(&f.dot, TriggerKind::Chat);
    told.parent_run_id = Some(lead.id.clone());
    told.root_run_id = Some(lead.root_run_id.clone());
    told.workflow_step_run_id = Some(step.id.clone());
    let reported = vec![one.id.clone()];
    let run = f.store.report_step_members(&told, &reported).await.unwrap();
    assert_eq!(run.root_run_id, lead.id);
    let members = f.store.step_members(&step.id).await.unwrap();
    assert_eq!(
        members.iter().map(|m| m.reported).collect::<Vec<_>>(),
        [true, false]
    );
    let both = vec![two.id.clone(), one.id.clone()];
    assert!(matches!(
        f.store.report_step_members(&told, &both).await,
        Err(Error::Conflict(_))
    ));
    // Nothing of the refused one was stored: the second member is still to report, and the
    // thread has no third run.
    assert!(!f.store.step_members(&step.id).await.unwrap()[1].reported);
    assert_eq!(f.store.list_thread(&lead.id).await.unwrap().len(), 2);
    // A step that has ended takes in no member.
    f.store
        .set_step_run(
            &step.id,
            &StepRunChange {
                status: Some(StepRunStatus::Cancelled),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        f.store.add_step_member(&new, "Part three.").await,
        Err(Error::Conflict(_))
    ));
    assert_eq!(f.store.step_members(&step.id).await.unwrap().len(), 2);
}

/// The ids `of`, as the store gives them.
fn ids(of: &[&String]) -> Vec<String> {
    of.iter().map(|id| id.to_string()).collect()
}

impl Fixture {
    /// A workflow run of `new`, started a moment after the one before: runs started in one
    /// millisecond would sort by their random ids.
    async fn started(&self, new: &NewWorkflowRun) -> WorkflowRun {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        self.store.create_workflow_run(new).await.unwrap()
    }

    /// Moves the workflow run `id` to `status`.
    async fn now(&self, id: &str, status: WorkflowRunStatus) {
        self.store
            .set_workflow_run(
                id,
                &WorkflowRunChange {
                    status: Some(status),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }

    /// The ids of the runs `filter` picks, in its order.
    async fn picked(&self, filter: WorkflowRunFilter) -> Vec<String> {
        let runs = self.store.find_workflow_runs(&filter).await.unwrap();
        runs.into_iter().map(|wr| wr.id).collect()
    }

    /// The ids of the workflow runs the dot runs `origins` started, in their order.
    async fn from(&self, origins: &[&String]) -> Vec<String> {
        let origins: Vec<String> = origins.iter().map(|id| id.to_string()).collect();
        let runs = self.store.workflow_runs_from(&origins).await.unwrap();
        runs.into_iter().map(|wr| wr.id).collect()
    }

    /// A chat run of the fixture's dot.
    async fn chat(&self) -> String {
        self.store
            .create_run(&NewRun::new(&self.dot, TriggerKind::Chat))
            .await
            .unwrap()
            .id
    }
}

#[tokio::test]
async fn workflow_runs_are_found_by_team_workflow_status_and_limit_newest_first() {
    let f = fixture().await;
    let other = f
        .store
        .create_workflow(
            &f.new.team_id,
            &dots_core::model::WorkflowSpec {
                name: "Other".into(),
                definition: f.new.definition.clone(),
                enabled: true,
            },
        )
        .await
        .unwrap();
    let of_other = NewWorkflowRun {
        workflow_id: other.id.clone(),
        ..f.new.clone()
    };
    let running = f.started(&f.new).await.id;
    let waiting = f.started(&f.new).await.id;
    f.now(&waiting, WorkflowRunStatus::Waiting).await;
    let done = f.started(&f.new).await.id;
    f.now(&done, WorkflowRunStatus::Succeeded).await;
    let elsewhere = f.started(&of_other).await.id;

    let all = ids(&[&elsewhere, &done, &waiting, &running]);
    assert_eq!(f.picked(WorkflowRunFilter::default()).await, all);
    let team = WorkflowRunFilter {
        team_id: Some(f.new.team_id.clone()),
        ..Default::default()
    };
    assert_eq!(f.picked(team.clone()).await, all);
    let fix = WorkflowRunFilter {
        workflow_id: Some(f.new.workflow_id.clone()),
        ..team
    };
    assert_eq!(
        f.picked(fix.clone()).await,
        ids(&[&done, &waiting, &running])
    );
    let in_flight = WorkflowRunFilter {
        statuses: vec![WorkflowRunStatus::Running, WorkflowRunStatus::Waiting],
        ..Default::default()
    };
    assert_eq!(
        f.picked(in_flight.clone()).await,
        ids(&[&elsewhere, &waiting, &running])
    );
    assert_eq!(
        f.picked(WorkflowRunFilter {
            limit: Some(1),
            ..in_flight
        })
        .await,
        ids(&[&elsewhere])
    );
    assert_eq!(
        f.picked(WorkflowRunFilter {
            statuses: vec![WorkflowRunStatus::Waiting],
            ..fix.clone()
        })
        .await,
        ids(&[&waiting])
    );
    assert_eq!(
        f.picked(WorkflowRunFilter {
            limit: Some(2),
            ..fix
        })
        .await,
        ids(&[&done, &waiting])
    );
    assert!(f
        .picked(WorkflowRunFilter {
            team_id: Some("no-team".into()),
            ..Default::default()
        })
        .await
        .is_empty());
    assert!(f
        .picked(WorkflowRunFilter {
            statuses: vec![WorkflowRunStatus::Failed],
            ..Default::default()
        })
        .await
        .is_empty());
    // A filter as the app sends it: every field may be left out.
    let sent: WorkflowRunFilter =
        serde_json::from_value(json!({"statuses": ["waiting"], "limit": 5})).unwrap();
    assert_eq!(sent.team_id, None);
    assert_eq!(f.picked(sent).await, ids(&[&waiting]));
    // The team's list is the filter with a team and a limit.
    let listed = f
        .store
        .list_workflow_runs(Some(&f.new.team_id), 2)
        .await
        .unwrap();
    let listed: Vec<_> = listed.iter().map(|wr| &wr.id).collect();
    assert_eq!(listed, [&elsewhere, &done]);
}

#[tokio::test]
async fn the_runs_a_chat_started_are_found_by_its_runs_oldest_first() {
    let f = fixture().await;
    let (one, two, quiet) = (f.chat().await, f.chat().await, f.chat().await);
    let from = |origin: &str| NewWorkflowRun {
        trigger: WorkflowTrigger::Chat,
        origin_run_id: Some(origin.to_string()),
        ..f.new.clone()
    };
    let first = f.started(&from(&one)).await.id;
    let manual = f.started(&f.new).await.id;
    let second = f.started(&from(&two)).await.id;
    let again = f.started(&from(&one)).await.id;
    f.now(&again, WorkflowRunStatus::Cancelled).await;

    assert_eq!(
        f.from(&[&two, &one, &quiet]).await,
        ids(&[&first, &second, &again])
    );
    assert_eq!(f.from(&[&two]).await, ids(&[&second]));
    assert!(f.from(&[&quiet]).await.is_empty());
    assert!(
        f.from(&[&manual]).await.is_empty(),
        "a workflow run's id is no origin"
    );
    assert!(f.from(&[]).await.is_empty());
}

#[tokio::test]
async fn a_workflow_says_how_many_runs_are_in_flight_and_which_started_last() {
    let f = fixture().await;
    let id = f.new.workflow_id.clone();
    let fix = f.store.get_workflow(&id).await.unwrap();
    assert_eq!((fix.runs_in_flight, fix.last_run), (0, None));

    let first = f.started(&f.new).await;
    let fix = f.store.get_workflow(&id).await.unwrap();
    assert_eq!(fix.runs_in_flight, 1);
    assert_eq!(
        fix.last_run,
        Some(LastRun {
            id: first.id.clone(),
            status: WorkflowRunStatus::Running,
            started_at: first.started_at.clone(),
            test: false,
        })
    );
    let test = f
        .started(&NewWorkflowRun {
            test: true,
            ..f.new.clone()
        })
        .await;
    f.now(&test.id, WorkflowRunStatus::Waiting).await;
    let listed = f.store.list_workflows(&f.new.team_id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].runs_in_flight, 2);
    assert_eq!(
        listed[0].last_run,
        Some(LastRun {
            id: test.id.clone(),
            status: WorkflowRunStatus::Waiting,
            started_at: test.started_at.clone(),
            test: true,
        })
    );
    // Ended runs are not in flight; the newest stays the last run, whatever its status.
    f.now(&first.id, WorkflowRunStatus::Succeeded).await;
    f.now(&test.id, WorkflowRunStatus::Failed).await;
    let saved = f.store.update_workflow(&id, &fix.spec).await.unwrap();
    assert_eq!(saved.runs_in_flight, 0);
    let last = saved.last_run.clone().expect("a last run");
    assert_eq!((last.id, last.status), (test.id, WorkflowRunStatus::Failed));
    // Serialized beside the rest of the workflow.
    let value = serde_json::to_value(&saved).unwrap();
    assert_eq!(value["runs_in_flight"], json!(0));
    assert_eq!(value["last_run"]["status"], json!("failed"));
    assert_eq!(value["last_run"]["test"], json!(true));
}

impl Fixture {
    /// Puts the workflow run `id` in wait on `on`, at the step `step`.
    async fn wait(&self, id: &str, on: WaitingOn, step: Option<&str>) {
        self.store
            .set_workflow_run(
                id,
                &WorkflowRunChange {
                    status: Some(WorkflowRunStatus::Waiting),
                    waiting_on: Some(Some(on)),
                    current_step: Some(step.map(str::to_string)),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }

    /// A run of the fixture's dot for the step run `step_run_id`, with `branch`, ended with
    /// `status`.
    async fn step_run_ended(
        &self,
        step_run_id: &str,
        branch: Option<&str>,
        status: RunStatus,
    ) -> dots_core::model::Run {
        let mut new = NewRun::new(&self.dot, TriggerKind::Manual);
        new.workflow_step_run_id = Some(step_run_id.to_string());
        new.branch = branch.map(str::to_string);
        let run = self.store.create_run(&new).await.unwrap();
        self.store
            .finish_run(&run.id, status, None, None)
            .await
            .unwrap()
    }
}

/// The Inbox lists every workflow run that waits for the person, newest first, with its team,
/// its workflow, its task's first line, what it waits on and at which step, and the dot whose
/// chat started it. A run that runs or has ended is not listed.
#[tokio::test]
async fn the_waiting_workflow_runs_are_listed_for_the_inbox_newest_first() {
    let f = fixture().await;
    let chatter = f
        .store
        .create_dot(&spec("chatter"), None, "default")
        .await
        .unwrap()
        .id;
    let chat = f
        .store
        .create_run(&NewRun::new(&chatter, TriggerKind::Chat))
        .await
        .unwrap();
    let _running = f.started(&f.new).await;
    let check = f.started(&f.new).await;
    f.wait(&check.id, WaitingOn::Check, Some("a")).await;
    let budget = f
        .started(&NewWorkflowRun {
            task: "  Ship the fix.\nThen tell the team.".into(),
            origin_run_id: Some(chat.id.clone()),
            test: true,
            ..f.new.clone()
        })
        .await;
    f.wait(&budget.id, WaitingOn::Budget, None).await;
    let ended = f.started(&f.new).await;
    f.wait(&ended.id, WaitingOn::StepFailed, Some("a")).await;
    f.now(&ended.id, WorkflowRunStatus::Failed).await;

    let item = |wr: &WorkflowRun| WaitingWorkflowRun {
        id: wr.id.clone(),
        team_id: f.new.team_id.clone(),
        team_name: "T".into(),
        workflow_id: Some(f.new.workflow_id.clone()),
        workflow_name: Some("Fix".into()),
        task: "The tray icon is invisible.".into(),
        waiting_on: WaitingOn::Check,
        step_name: Some("Work".into()),
        origin_dot_id: None,
        test: false,
        started_at: wr.started_at.clone(),
    };
    let budget_item = WaitingWorkflowRun {
        task: "Ship the fix.".into(),
        waiting_on: WaitingOn::Budget,
        step_name: None,
        origin_dot_id: Some(chatter.clone()),
        test: true,
        ..item(&budget)
    };
    assert_eq!(
        f.store.waiting_workflow_runs().await.unwrap(),
        vec![budget_item.clone(), item(&check)]
    );
    let value = serde_json::to_value(&budget_item).unwrap();
    assert_eq!(value["waiting_on"], json!("budget"));
    assert_eq!(value["step_name"], json!(null));
    assert_eq!(value["origin_dot_id"], json!(chatter));

    // A deleted workflow and a deleted chat leave the run listed, without them.
    f.store.delete_workflow(&f.new.workflow_id).await.unwrap();
    f.store.delete_dot(&chatter).await.unwrap();
    let listed = f.store.waiting_workflow_runs().await.unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|w| (
                w.id.as_str(),
                w.workflow_id.as_deref(),
                w.workflow_name.as_deref()
            ))
            .collect::<Vec<_>>(),
        vec![
            (budget.id.as_str(), None, None),
            (check.id.as_str(), None, None)
        ]
    );
    assert_eq!(listed[0].origin_dot_id, None);
}

/// A waiting workflow run that a chat started needs the chat's dot, only while it waits; one
/// started another way needs no dot.
#[tokio::test]
async fn a_waiting_workflow_run_needs_the_dot_whose_chat_started_it() {
    let f = fixture().await;
    let other = f
        .store
        .create_dot(&spec("other"), None, "default")
        .await
        .unwrap()
        .id;
    let chat = f.chat().await;
    let started = f
        .started(&NewWorkflowRun {
            origin_run_id: Some(chat),
            ..f.new.clone()
        })
        .await;
    let manual = f.started(&f.new).await;
    f.wait(&manual.id, WaitingOn::Check, Some("a")).await;
    assert_eq!(f.store.dot_counts(&f.dot).await.unwrap(), (0, 0));

    f.wait(&started.id, WaitingOn::Check, Some("a")).await;
    assert_eq!(f.store.dot_counts(&f.dot).await.unwrap(), (1, 1));
    assert_eq!(f.store.dot_counts(&other).await.unwrap(), (0, 0));
    // Approved, it runs again; held by the budget, it waits again; stopped, it has ended.
    f.now(&started.id, WorkflowRunStatus::Running).await;
    assert_eq!(f.store.dot_counts(&f.dot).await.unwrap(), (0, 0));
    f.wait(&started.id, WaitingOn::Budget, Some("a")).await;
    assert_eq!(f.store.dot_counts(&f.dot).await.unwrap(), (1, 1));
    f.now(&started.id, WorkflowRunStatus::Cancelled).await;
    assert_eq!(f.store.dot_counts(&f.dot).await.unwrap(), (0, 0));
}

/// A run of a workflow step is never a run to review, failed or with a branch, and nor is a
/// run that continues it while the step is open; the person's own runs still are. The step's
/// approvals still need its dot.
#[tokio::test]
async fn a_step_s_runs_are_never_runs_to_review() {
    let f = fixture().await;
    let wr = f.started(&f.new).await;
    let step = f
        .store
        .create_step_run(&wr.id, "a", 1, StepRunStatus::Running)
        .await
        .unwrap();
    let failed = f.step_run_ended(&step.id, None, RunStatus::Failed).await;
    f.step_run_ended(&step.id, Some("dots/work-1"), RunStatus::Succeeded)
        .await;
    let mut reply = NewRun::new(&f.dot, TriggerKind::Chat);
    reply.parent_run_id = Some(failed.id.clone());
    reply.root_run_id = Some(failed.root_run_id.clone());
    let reply = f.store.create_run(&reply).await.unwrap();
    assert_eq!(reply.workflow_step_run_id, Some(step.id.clone()));
    f.store
        .finish_run(&reply.id, RunStatus::Failed, None, Some("down"))
        .await
        .unwrap();
    let own = f
        .store
        .create_run(&NewRun::new(&f.dot, TriggerKind::Manual))
        .await
        .unwrap();
    f.store
        .finish_run(&own.id, RunStatus::Failed, None, Some("down"))
        .await
        .unwrap();
    let listed: Vec<String> = f
        .store
        .unreviewed_thread_ends()
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    assert_eq!(listed, vec![own.id]);

    let mut parked = NewRun::new(&f.dot, TriggerKind::Manual);
    parked.workflow_step_run_id = Some(step.id.clone());
    let parked = f.store.create_run(&parked).await.unwrap();
    f.store
        .create_approval(&parked.id, "Bash", &json!({"command": "ls"}))
        .await
        .unwrap();
    assert_eq!(f.store.dot_counts(&f.dot).await.unwrap(), (1, 1));
}

/// A dot run queued a moment after the one before (runs queued in one millisecond would sort by
/// their random ids), working for `step_run_id`, continuing `parent` when one is given.
async fn queued(store: &Store, dot: &str, step_run_id: &str, parent: Option<&Run>) -> Run {
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let mut new = NewRun::new(dot, TriggerKind::Workflow);
    new.workflow_step_run_id = Some(step_run_id.to_string());
    new.parent_run_id = parent.map(|p| p.id.clone());
    new.root_run_id = parent.map(|p| p.root_run_id.clone());
    store.create_run(&new).await.unwrap()
}

/// The run page's link to `run`, a run of the dot named `dot_name`, as the store reads it now.
async fn link(store: &Store, run: &Run, dot_name: &str) -> StepDotRun {
    let run = store.get_run(&run.id).await.unwrap();
    StepDotRun {
        step_run_id: run.workflow_step_run_id.clone().unwrap(),
        run_id: run.id.clone(),
        root_run_id: run.root_run_id.clone(),
        dot_id: run.dot_id.clone(),
        dot_name: dot_name.into(),
        status: run.status,
        started_at: run.started_at.clone(),
    }
}

/// The run page links each step to its own dot runs: every run of the workflow run's step runs,
/// across attempts, with a resume child in its parent's thread and a delegate member's run,
/// oldest first, each with its dot's name and its state. Another workflow run's runs and a plain
/// run are left out. A workflow run with no dot run gives none, an unknown one `NotFound`.
#[tokio::test]
async fn a_workflow_run_s_dot_runs_are_those_of_its_step_runs_oldest_first() {
    let f = fixture().await;
    let reviewer = f
        .store
        .create_dot(&spec("reviewer"), None, "default")
        .await
        .unwrap()
        .id;
    let wr = f.started(&f.new).await;
    assert_eq!(f.store.workflow_run_dot_runs(&wr.id).await.unwrap(), []);
    let first = f
        .store
        .create_step_run(&wr.id, "a", 1, StepRunStatus::Running)
        .await
        .unwrap();
    assert_eq!(f.store.workflow_run_dot_runs(&wr.id).await.unwrap(), []);

    let lead = queued(&f.store, &f.dot, &first.id, None).await;
    assert!(f.store.claim_run(&lead.id).await.unwrap());
    let child = queued(&f.store, &f.dot, &first.id, Some(&lead)).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    let mut member = NewRun::new(&reviewer, TriggerKind::Workflow);
    member.workflow_step_run_id = Some(first.id.clone());
    let member = f
        .store
        .add_step_member(&member, "Check it on Windows 10.")
        .await
        .unwrap();
    f.store
        .set_step_run(
            &first.id,
            &StepRunChange {
                status: Some(StepRunStatus::Failed),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let second = f
        .store
        .create_step_run(&wr.id, "a", 2, StepRunStatus::Running)
        .await
        .unwrap();
    let retry = queued(&f.store, &f.dot, &second.id, None).await;
    f.store
        .finish_run(&retry.id, RunStatus::Succeeded, None, None)
        .await
        .unwrap();

    // Not the page's: another workflow run's step run, and a run of no step.
    let other = f.started(&f.new).await;
    let theirs = f
        .store
        .create_step_run(&other.id, "a", 1, StepRunStatus::Running)
        .await
        .unwrap();
    let theirs = queued(&f.store, &f.dot, &theirs.id, None).await;
    f.store
        .create_run(&NewRun::new(&f.dot, TriggerKind::Manual))
        .await
        .unwrap();

    let got = f.store.workflow_run_dot_runs(&wr.id).await.unwrap();
    assert_eq!(
        got,
        [
            link(&f.store, &lead, "dev").await,
            link(&f.store, &child, "dev").await,
            link(&f.store, &member, "reviewer").await,
            link(&f.store, &retry, "dev").await,
        ]
    );
    let states: Vec<_> = got
        .iter()
        .map(|r| (r.step_run_id.as_str(), r.root_run_id.as_str(), r.status))
        .collect();
    assert_eq!(
        states,
        [
            (first.id.as_str(), lead.id.as_str(), RunStatus::Running),
            (first.id.as_str(), lead.id.as_str(), RunStatus::Queued),
            (first.id.as_str(), member.id.as_str(), RunStatus::Queued),
            (second.id.as_str(), retry.id.as_str(), RunStatus::Succeeded),
        ]
    );
    assert!(got[0].started_at.is_some());
    assert_eq!(got[1].started_at, None);
    assert_eq!(
        serde_json::to_value(&got[2]).unwrap(),
        json!({
            "step_run_id": first.id,
            "run_id": member.id,
            "root_run_id": member.id,
            "dot_id": reviewer,
            "dot_name": "reviewer",
            "status": "queued",
            "started_at": null,
        })
    );
    let theirs_too = f.store.workflow_run_dot_runs(&other.id).await.unwrap();
    assert_eq!(
        theirs_too.iter().map(|r| &r.run_id).collect::<Vec<_>>(),
        [&theirs.id]
    );
    let err = f.store.workflow_run_dot_runs("nope").await.unwrap_err();
    assert!(
        matches!(&err, Error::NotFound(m) if m == "workflow run nope"),
        "{err:?}"
    );
}
