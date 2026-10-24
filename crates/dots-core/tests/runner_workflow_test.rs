mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store, wait_status};
use dots_core::engine::scripted::{ScriptedEngine, Step};
use dots_core::engine::{Engine, EngineEvent};
use dots_core::events::new_bus;
use dots_core::model::{
    Dot, EngineKind, MemberSpec, NewRun, NewWorkflowRun, ProjectSpec, Run, RunStatus,
    StepRunStatus, TeamSpec, TriggerKind, WorkflowSpec, WorkflowTrigger, WorkspaceMode,
};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use dots_core::workflow::{Budget, Definition, Step as FlowStep, StepKind, Trigger};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn runner_with(store: &Store, dir: &TempDir, engine: Arc<dyn Engine>, max: usize) -> Arc<Runner> {
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Claude, engine);
    Runner::new(
        store.clone(),
        new_bus(),
        engines,
        RunnerConfig {
            max_concurrent: max,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(200),
            plugins_dir: dir.path().join("plugins"),
        },
    )
}

async fn folder_dot(store: &Store, dir: &TempDir, name: &str) -> Dot {
    let mut s = spec(name);
    s.workdir = dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    store.create_dot(&s).await.unwrap()
}

/// A step run of a one-step workflow of a team that has `dot`, for runs to belong to.
async fn step_run(store: &Store, dir: &TempDir, dot: &Dot) -> String {
    let team = store
        .create_team(&TeamSpec {
            name: format!("team-{}", dot.spec.name),
            description: String::new(),
            members: vec![MemberSpec {
                dot_id: dot.id.clone(),
                role: String::new(),
            }],
        })
        .await
        .unwrap();
    let definition = Definition {
        trigger: Trigger::Manual,
        steps: vec![FlowStep {
            id: "a".into(),
            name: "Work".into(),
            kind: StepKind::Agent {
                member: dot.id.clone(),
                job: "Work.".into(),
            },
        }],
        budget: Budget::default(),
    };
    let workflow = store
        .create_workflow(
            &team.id,
            &WorkflowSpec {
                name: "W".into(),
                definition: definition.clone(),
                enabled: true,
            },
        )
        .await
        .unwrap();
    let project = ProjectSpec {
        name: "P".into(),
        repo_path: dir.path().to_string_lossy().to_string(),
        default_branch: None,
        remote: "origin".into(),
    };
    let wr = store
        .create_workflow_run(&NewWorkflowRun {
            workflow_id: workflow.id,
            team_id: team.id,
            definition,
            project_id: "p".into(),
            project,
            task: "t".into(),
            trigger: WorkflowTrigger::Manual,
            origin_run_id: None,
            test: false,
        })
        .await
        .unwrap();
    store
        .create_step_run(&wr.id, "a", 1, StepRunStatus::Running)
        .await
        .unwrap()
        .id
}

async fn queue(runner: &Runner, dot: &Dot, step: Option<&str>) -> Run {
    let mut new = NewRun::new(
        &dot.id,
        if step.is_some() {
            TriggerKind::Workflow
        } else {
            TriggerKind::Manual
        },
    );
    new.workflow_step_run_id = step.map(str::to_string);
    runner.enqueue(new).await.unwrap()
}

fn slow() -> Arc<ScriptedEngine> {
    Arc::new(ScriptedEngine::new(vec![
        Step::Sleep(Duration::from_millis(1500)),
        Step::Emit(EngineEvent::Finished {
            summary: "done".into(),
        }),
    ]))
}

#[tokio::test]
async fn two_workflow_runs_of_one_dot_run_at_once() {
    let (dir, store) = temp_store().await;
    let runner = runner_with(&store, &dir, slow(), 4);
    runner.spawn_dispatcher(CancellationToken::new());
    let dev = folder_dot(&store, &dir, "dev").await;
    let step = step_run(&store, &dir, &dev).await;
    let a = queue(&runner, &dev, Some(&step)).await;
    let b = queue(&runner, &dev, Some(&step)).await;
    wait_status(&store, &a.id, RunStatus::Running, 5).await;
    wait_status(&store, &b.id, RunStatus::Running, 5).await;
    // Both at once: the first is still running when the second starts.
    assert_eq!(
        store.get_run(&a.id).await.unwrap().status,
        RunStatus::Running
    );
    wait_status(&store, &a.id, RunStatus::Succeeded, 5).await;
    wait_status(&store, &b.id, RunStatus::Succeeded, 5).await;
}

#[tokio::test]
async fn ordinary_runs_wait_only_for_ordinary_runs() {
    let (dir, store) = temp_store().await;
    let runner = runner_with(&store, &dir, slow(), 4);
    runner.spawn_dispatcher(CancellationToken::new());
    let dev = folder_dot(&store, &dir, "dev").await;
    let step = step_run(&store, &dir, &dev).await;
    let first = queue(&runner, &dev, None).await;
    wait_status(&store, &first.id, RunStatus::Running, 5).await;
    let second = queue(&runner, &dev, None).await;
    let flow = queue(&runner, &dev, Some(&step)).await;
    // The workflow run starts beside the ordinary one; the second ordinary one waits.
    wait_status(&store, &flow.id, RunStatus::Running, 5).await;
    assert_eq!(
        store.get_run(&first.id).await.unwrap().status,
        RunStatus::Running
    );
    assert_eq!(
        store.get_run(&second.id).await.unwrap().status,
        RunStatus::Queued
    );
    wait_status(&store, &first.id, RunStatus::Succeeded, 5).await;
    wait_status(&store, &second.id, RunStatus::Succeeded, 5).await;
}

#[tokio::test]
async fn the_concurrency_limit_still_holds_for_workflow_runs() {
    let (dir, store) = temp_store().await;
    let runner = runner_with(&store, &dir, slow(), 1);
    runner.spawn_dispatcher(CancellationToken::new());
    let dev = folder_dot(&store, &dir, "dev").await;
    let step = step_run(&store, &dir, &dev).await;
    let a = queue(&runner, &dev, Some(&step)).await;
    let b = queue(&runner, &dev, Some(&step)).await;
    wait_status(&store, &a.id, RunStatus::Running, 5).await;
    assert_eq!(
        store.get_run(&b.id).await.unwrap().status,
        RunStatus::Queued
    );
    wait_status(&store, &b.id, RunStatus::Succeeded, 5).await;
}
