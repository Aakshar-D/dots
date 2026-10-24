mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::{git_out, init_repo, spec, temp_store, wait_status};
use dots_core::engine::scripted::{ScriptedEngine, Step};
use dots_core::engine::{Engine, EngineEvent, RunContext};
use dots_core::events::new_bus;
use dots_core::model::{
    Dot, EngineKind, MemberSpec, NewRun, NewWorkflowRun, ProjectSpec, Run, RunStatus,
    StepRunStatus, TeamSpec, TriggerKind, WorkflowSpec, WorkflowTrigger, WorkspaceMode,
};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use dots_core::workflow::{Budget, Definition, Step as FlowStep, StepKind, Trigger};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::mpsc;
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
    step_run_in(store, dot, dir.path(), None).await
}

/// `step_run`, with the project's repo at `repo` and its default branch.
async fn step_run_in(
    store: &Store,
    dot: &Dot,
    repo: &std::path::Path,
    default_branch: Option<&str>,
) -> String {
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
        repo_path: repo.to_string_lossy().to_string(),
        default_branch: default_branch.map(str::to_string),
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

/// Writes `file` into its workspace (when given), keeps each prompt, and finishes.
struct Writer {
    file: Option<(&'static str, &'static str)>,
    prompts: std::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl Engine for Writer {
    async fn start(&self, ctx: RunContext) -> dots_core::Result<mpsc::Receiver<EngineEvent>> {
        self.prompts.lock().unwrap().push(ctx.prompt.clone());
        if let Some((name, text)) = self.file {
            std::fs::write(ctx.workspace.join(name), text).unwrap();
        }
        let (tx, rx) = mpsc::channel(4);
        tx.send(EngineEvent::Finished {
            summary: "Fixed the tray icon.\nMore detail.".into(),
        })
        .await
        .unwrap();
        Ok(rx)
    }
}

fn writer(file: Option<(&'static str, &'static str)>) -> Arc<Writer> {
    Arc::new(Writer {
        file,
        prompts: std::sync::Mutex::new(Vec::new()),
    })
}

/// A dot in worktree mode whose own folder is a repo of its own, not the project's.
async fn worktree_dot(store: &Store, dir: &TempDir, name: &str) -> Dot {
    let own = dir.path().join(format!("{name}-own"));
    init_repo(&own);
    let mut s = spec(name);
    s.workdir = own.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Worktree;
    store.create_dot(&s).await.unwrap()
}

async fn queue_with(runner: &Runner, dot: &Dot, step: &str, payload: serde_json::Value) -> Run {
    let mut new = NewRun::new(&dot.id, TriggerKind::Workflow);
    new.workflow_step_run_id = Some(step.to_string());
    new.payload = Some(payload);
    runner.enqueue(new).await.unwrap()
}

#[tokio::test]
async fn a_step_works_in_a_worktree_of_the_project_from_its_start() {
    let (dir, store) = temp_store().await;
    let project = dir.path().join("project");
    init_repo(&project);
    git_out(&project, &["checkout", "-q", "-b", "feature"]);
    std::fs::write(project.join("feature.txt"), "x").unwrap();
    git_out(&project, &["add", "."]);
    git_out(&project, &["commit", "-q", "-m", "feature"]);
    git_out(&project, &["checkout", "-q", "main"]);
    let tip = git_out(&project, &["rev-parse", "feature"]);
    let engine = writer(Some(("fix.txt", "fixed")));
    let runner = runner_with(&store, &dir, engine.clone(), 2);
    runner.spawn_dispatcher(CancellationToken::new());
    let dev = worktree_dot(&store, &dir, "dev").await;
    let step = step_run_in(&store, &dev, &project, None).await;
    let run = queue_with(
        &runner,
        &dev,
        &step,
        json!({"workflow": {"start": "feature"}}),
    )
    .await;
    let run = wait_status(&store, &run.id, RunStatus::Succeeded, 10).await;
    assert_eq!(run.base_commit.as_deref(), Some(tip.as_str()));
    let branch = run.branch.clone().unwrap();
    assert_eq!(branch, format!("dots/dev/{}", run.id));
    // The branch is the project's, with the step's work committed on top of `feature`.
    assert_eq!(
        git_out(&project, &["rev-parse", &format!("{branch}~1")]),
        tip
    );
    assert_eq!(
        git_out(&project, &["log", "-1", "--format=%s", &branch]),
        "Work: Fixed the tray icon."
    );
    assert_eq!(
        git_out(&project, &["show", &format!("{branch}:fix.txt")]),
        "fixed"
    );
    // The worktree has a commit, so it is kept for review.
    assert!(std::path::Path::new(run.workspace_path.as_deref().unwrap()).is_dir());
}

#[tokio::test]
async fn the_commit_skips_the_repos_hooks() {
    let (dir, store) = temp_store().await;
    let project = dir.path().join("project");
    init_repo(&project);
    // A hook that would refuse every commit.
    let hooks = dir.path().join("hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::write(hooks.join("pre-commit"), "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::write(hooks.join("commit-msg"), "#!/bin/sh\nexit 1\n").unwrap();
    git_out(
        &project,
        &["config", "core.hooksPath", &hooks.to_string_lossy()],
    );
    let runner = runner_with(&store, &dir, writer(Some(("fix.txt", "fixed"))), 2);
    runner.spawn_dispatcher(CancellationToken::new());
    let dev = worktree_dot(&store, &dir, "dev").await;
    let step = step_run_in(&store, &dev, &project, None).await;
    let run = queue_with(&runner, &dev, &step, json!({})).await;
    let run = wait_status(&store, &run.id, RunStatus::Succeeded, 10).await;
    let branch = run.branch.unwrap();
    assert_eq!(
        git_out(&project, &["show", &format!("{branch}:fix.txt")]),
        "fixed"
    );
}

#[tokio::test]
async fn a_step_with_nothing_to_commit_leaves_no_worktree() {
    let (dir, store) = temp_store().await;
    let project = dir.path().join("project");
    init_repo(&project);
    let runner = runner_with(&store, &dir, writer(None), 2);
    runner.spawn_dispatcher(CancellationToken::new());
    let dev = worktree_dot(&store, &dir, "dev").await;
    let step = step_run_in(&store, &dev, &project, Some("main")).await;
    let run = queue_with(&runner, &dev, &step, json!({})).await;
    let run = wait_status(&store, &run.id, RunStatus::Succeeded, 10).await;
    assert!(!std::path::Path::new(run.workspace_path.as_deref().unwrap()).exists());
}

#[tokio::test]
async fn a_start_that_is_not_in_the_repo_fails_the_run() {
    let (dir, store) = temp_store().await;
    let project = dir.path().join("project");
    init_repo(&project);
    let runner = runner_with(&store, &dir, writer(None), 2);
    runner.spawn_dispatcher(CancellationToken::new());
    let dev = worktree_dot(&store, &dir, "dev").await;
    let step = step_run_in(&store, &dev, &project, None).await;
    let run = queue_with(&runner, &dev, &step, json!({"workflow": {"start": "nope"}})).await;
    let run = wait_status(&store, &run.id, RunStatus::Failed, 10).await;
    assert!(
        run.error
            .as_deref()
            .unwrap()
            .contains("nope is not a branch or commit of the repo"),
        "{:?}",
        run.error
    );
}

#[tokio::test]
async fn a_step_run_gets_the_workflow_section_in_its_prompt() {
    let (dir, store) = temp_store().await;
    let project = dir.path().join("project");
    init_repo(&project);
    let engine = writer(None);
    let runner = runner_with(&store, &dir, engine.clone(), 2);
    runner.spawn_dispatcher(CancellationToken::new());
    let dev = worktree_dot(&store, &dir, "dev").await;
    let step = step_run_in(&store, &dev, &project, None).await;
    let section = "## Workflow\n\nTeam T. Your job: fix it.";
    let run = queue_with(
        &runner,
        &dev,
        &step,
        json!({"workflow": {"section": section}}),
    )
    .await;
    wait_status(&store, &run.id, RunStatus::Succeeded, 10).await;
    let prompt = engine.prompts.lock().unwrap()[0].clone();
    assert!(prompt.starts_with("Do the thing."), "{prompt}");
    assert!(
        prompt.contains("---\n\n## Workflow\n\nTeam T. Your job: fix it.\n\n## Standing rules"),
        "{prompt}"
    );
    assert!(!prompt.contains("## Trigger"), "{prompt}");
}
