mod common;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{git_out, init_repo, spec, temp_store};
use dots_core::checkins::CheckIns;
use dots_core::engine::{Engine, EngineEvent, RunContext};
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::followups::FollowUps;
use dots_core::model::{
    Dot, EngineKind, MemberSpec, ProjectSpec, RunStatus, StepRunStatus, TeamSpec, WaitingOn,
    WorkflowRun, WorkflowRunStatus, WorkflowSpec, WorkflowTrigger, WorkspaceMode,
};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::scheduler::SystemClock;
use dots_core::store::Store;
use dots_core::workflow::{Budget, Definition, Step, StepKind, Trigger};
use dots_core::workflows::{StartRequest, Workflows};
use dots_core::Error;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// What a dot's next run does.
#[derive(Clone)]
enum Act {
    /// Finishes with this summary.
    Finish(&'static str),
    /// Writes a file into its workspace, then finishes with this summary.
    Write(&'static str, &'static str),
    /// Fails with this error.
    Fail(&'static str),
    /// Asks the person this question, then finishes.
    Ask(&'static str),
    /// Runs until it is cancelled.
    Hold,
}

/// Plays each dot's acts in order, one per run (the last one again once they run out), and
/// keeps each run's prompt.
struct Team {
    store: Store,
    acts: Mutex<HashMap<String, VecDeque<Act>>>,
    prompts: Mutex<Vec<(String, String)>>,
}

impl Team {
    fn act(&self, dot: &str) -> Act {
        let mut acts = self.acts.lock().unwrap();
        let queue = acts.entry(dot.to_string()).or_default();
        match queue.len() {
            0 => Act::Finish("done"),
            1 => queue[0].clone(),
            _ => queue.pop_front().unwrap(),
        }
    }

    fn set(&self, dot: &str, acts: Vec<Act>) {
        self.acts
            .lock()
            .unwrap()
            .insert(dot.to_string(), acts.into());
    }

    fn prompts_of(&self, dot: &str) -> Vec<String> {
        self.prompts
            .lock()
            .unwrap()
            .iter()
            .filter(|(d, _)| d == dot)
            .map(|(_, p)| p.clone())
            .collect()
    }
}

#[async_trait::async_trait]
impl Engine for Team {
    async fn start(&self, ctx: RunContext) -> dots_core::Result<mpsc::Receiver<EngineEvent>> {
        let name = ctx.dot.spec.name.clone();
        self.prompts
            .lock()
            .unwrap()
            .push((name.clone(), ctx.prompt.clone()));
        let act = self.act(&name);
        let store = self.store.clone();
        let (tx, rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let session = format!("session-{}", ctx.run_id);
            let _ = tx
                .send(EngineEvent::SessionStarted {
                    session_id: session,
                })
                .await;
            let end = match act {
                Act::Finish(s) => EngineEvent::Finished { summary: s.into() },
                Act::Write(file, s) => {
                    std::fs::write(ctx.workspace.join(file), s).unwrap();
                    EngineEvent::Finished { summary: s.into() }
                }
                Act::Fail(e) => EngineEvent::Failed { error: e.into() },
                Act::Ask(q) => {
                    store
                        .add_question(&ctx.dot.id, &ctx.run_id, q, None)
                        .await
                        .unwrap();
                    EngineEvent::Finished {
                        summary: "I asked.".into(),
                    }
                }
                Act::Hold => {
                    ctx.cancel.cancelled().await;
                    return;
                }
            };
            let _ = tx.send(end).await;
        });
        Ok(rx)
    }
}

struct World {
    dir: TempDir,
    store: Store,
    bus: Bus,
    runner: Arc<Runner>,
    engine: Arc<Team>,
    workflows: Arc<Workflows>,
    repo: PathBuf,
    team: String,
    project: String,
    helper: Dot,
    dev: Dot,
}

async fn worktree_dot(store: &Store, dir: &Path, name: &str) -> Dot {
    let own = dir.join(format!("{name}-own"));
    init_repo(&own);
    let mut s = spec(name);
    s.workdir = own.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Worktree;
    store.create_dot(&s).await.unwrap()
}

async fn world() -> World {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let engine = Arc::new(Team {
        store: store.clone(),
        acts: Mutex::new(HashMap::new()),
        prompts: Mutex::new(Vec::new()),
    });
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Claude, engine.clone());
    let runner = Runner::new(
        store.clone(),
        bus.clone(),
        engines,
        RunnerConfig {
            max_concurrent: 4,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(200),
            plugins_dir: dir.path().join("plugins"),
        },
    );
    runner.spawn_dispatcher(CancellationToken::new());
    let workflows = Workflows::new(
        store.clone(),
        bus.clone(),
        runner.clone(),
        Arc::new(SystemClock),
    );
    workflows.spawn(CancellationToken::new());
    let repo = dir.path().join("project");
    init_repo(&repo);
    let helper = worktree_dot(&store, dir.path(), "helper").await;
    let dev = worktree_dot(&store, dir.path(), "dev").await;
    let team = store
        .create_team(&TeamSpec {
            name: "Dots management".into(),
            description: String::new(),
            members: vec![
                MemberSpec {
                    dot_id: helper.id.clone(),
                    role: "user helper".into(),
                },
                MemberSpec {
                    dot_id: dev.id.clone(),
                    role: "dev agent".into(),
                },
            ],
        })
        .await
        .unwrap()
        .id;
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
    World {
        dir,
        store,
        bus,
        runner,
        engine,
        workflows,
        repo,
        team,
        project,
        helper,
        dev,
    }
}

fn agent(id: &str, name: &str, member: &Dot, job: &str) -> Step {
    Step {
        id: id.into(),
        name: name.into(),
        kind: StepKind::Agent {
            member: member.id.clone(),
            job: job.into(),
        },
    }
}

fn check(id: &str) -> Step {
    Step {
        id: id.into(),
        name: "Your check".into(),
        kind: StepKind::Check {},
    }
}

impl World {
    async fn workflow(&self, name: &str, steps: Vec<Step>) -> String {
        self.store
            .create_workflow(
                &self.team,
                &WorkflowSpec {
                    name: name.into(),
                    definition: Definition {
                        trigger: Trigger::Manual,
                        steps,
                        budget: Budget::default(),
                    },
                    enabled: true,
                },
            )
            .await
            .unwrap()
            .id
    }

    async fn start(&self, workflow: &str) -> WorkflowRun {
        self.workflows
            .start(StartRequest {
                workflow_id: workflow.into(),
                project_id: self.project.clone(),
                task: "The tray icon is invisible in dark mode.".into(),
                trigger: WorkflowTrigger::Manual,
                origin_run_id: None,
                test: false,
            })
            .await
            .unwrap()
    }

    /// The standard flow: helper writes the task, dev fixes it, the person checks.
    async fn standard(&self) -> String {
        self.workflow(
            "Fix",
            vec![
                agent(
                    "helper",
                    "Write the task",
                    &self.helper,
                    "Turn the request into a task.",
                ),
                agent("dev", "Fix it", &self.dev, "Fix the bug."),
                check("check"),
            ],
        )
        .await
    }

    /// Waits until the workflow run satisfies `done`, or panics with its last state.
    async fn until(&self, id: &str, done: impl Fn(&WorkflowRun) -> bool) -> WorkflowRun {
        // Generous: under a full test run, git on Windows makes each step take seconds.
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let wr = self.store.get_workflow_run(id).await.unwrap();
            if done(&wr) {
                return wr;
            }
            if std::time::Instant::now() > deadline {
                let mut runs = Vec::new();
                for s in &wr.steps {
                    for r in self.store.runs_of_step_run(&s.id).await.unwrap() {
                        runs.push((s.step_id.clone(), r.status, r.error, r.ended_at));
                    }
                }
                panic!("workflow run {id} did not get there; runs {runs:?}; last state: {wr:#?}");
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }
}

fn waits_on(w: WaitingOn) -> impl Fn(&WorkflowRun) -> bool {
    move |wr| wr.status == WorkflowRunStatus::Waiting && wr.waiting_on == Some(w)
}

#[tokio::test]
async fn a_run_goes_through_its_steps_and_waits_for_the_check() {
    let w = world().await;
    w.engine.set(
        &w.helper.spec.name,
        vec![Act::Finish("Task: make the icon visible.")],
    );
    w.engine.set(
        &w.dev.spec.name,
        vec![Act::Write("icon.txt", "Made the icon visible.")],
    );
    let flow = w.standard().await;
    let started = w.start(&flow).await;
    assert_eq!(started.status, WorkflowRunStatus::Running);
    let wr = w.until(&started.id, waits_on(WaitingOn::Check)).await;
    assert_eq!(wr.current_step.as_deref(), Some("check"));
    let statuses: Vec<_> = wr
        .steps
        .iter()
        .map(|s| (s.step_id.as_str(), s.status))
        .collect();
    assert_eq!(
        statuses,
        [
            ("helper", StepRunStatus::Succeeded),
            ("dev", StepRunStatus::Succeeded),
            ("check", StepRunStatus::Waiting),
        ]
    );
    // The helper made no change, so it hands on no branch; the dev's branch has its commit.
    let helper = wr.steps[0].handoff.clone().unwrap();
    assert_eq!(helper.summary, "Task: make the icon visible.");
    assert!(helper.branches.is_empty());
    let dev = wr.steps[1].handoff.clone().unwrap();
    assert_eq!(dev.branches.len(), 1);
    assert_eq!(
        git_out(
            &w.repo,
            &["show", &format!("{}:icon.txt", dev.branches[0].branch)]
        ),
        "Made the icon visible."
    );
    assert_eq!(wr.spent_runs, 2);

    // The dev's prompt has the workflow, the task, its job and the helper's hand-off.
    let prompt = &w.engine.prompts_of("dev")[0];
    for part in [
        "## Workflow",
        "You are the dev agent of the team \"Dots management\", doing one step of the workflow \"Fix\".",
        "- Step: Fix it (2 of 3)",
        "### Task\n\nThe tray icon is invisible in dark mode.",
        "### Your job in this step\n\nFix the bug.",
        "### Earlier steps\n\n#### Write the task\n\nTask: make the icon visible.",
    ] {
        assert!(prompt.contains(part), "missing {part:?} in:\n{prompt}");
    }

    let done = w
        .workflows
        .approve_check(&wr.id, Some("looks good"))
        .await
        .unwrap();
    assert_eq!(done.status, WorkflowRunStatus::Succeeded);
    assert!(done.ended_at.is_some());
    let check = &done.steps[2];
    assert_eq!(check.status, StepRunStatus::Succeeded);
    assert_eq!(check.decision.as_ref().unwrap()["note"], "looks good");
    // The check passes on what it was shown: the dev's branch.
    assert_eq!(check.handoff.as_ref().unwrap().branches, dev.branches);
}

#[tokio::test]
async fn a_later_step_starts_from_the_earlier_steps_branch() {
    let w = world().await;
    w.engine.set(
        &w.dev.spec.name,
        vec![
            Act::Write("one.txt", "First."),
            Act::Write("two.txt", "Second."),
        ],
    );
    let flow = w
        .workflow(
            "Two",
            vec![
                agent("one", "First", &w.dev, "Start."),
                agent("two", "Second", &w.dev, "Go on."),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    let first = &wr.steps[0].handoff.as_ref().unwrap().branches[0].branch;
    let second = &wr.steps[1].handoff.as_ref().unwrap().branches[0].branch;
    // The second branch holds both steps' work.
    assert_eq!(
        git_out(&w.repo, &["show", &format!("{second}:one.txt")]),
        "First."
    );
    assert_eq!(
        git_out(&w.repo, &["show", &format!("{second}:two.txt")]),
        "Second."
    );
    assert_eq!(
        git_out(&w.repo, &["merge-base", "--is-ancestor", first, second]),
        ""
    );
    assert!(w.engine.prompts_of("dev")[1].contains(&format!("Branches: {first}")));
}

#[tokio::test]
async fn a_failed_step_waits_and_can_be_retried() {
    let w = world().await;
    w.engine.set(
        &w.dev.spec.name,
        vec![Act::Fail("compiler exploded"), Act::Finish("Fixed.")],
    );
    let flow = w
        .workflow("Fix", vec![agent("dev", "Fix it", &w.dev, "Fix.")])
        .await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, waits_on(WaitingOn::StepFailed)).await;
    assert_eq!(wr.error.as_deref(), Some("Fix it: compiler exploded"));
    assert_eq!(wr.steps[0].status, StepRunStatus::Failed);
    assert!(matches!(
        w.workflows.approve_check(&wr.id, None).await,
        Err(Error::Conflict(_))
    ));
    let retried = w.workflows.retry_step(&wr.id).await.unwrap();
    assert_eq!(retried.error, None);
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    let attempts: Vec<_> = wr.steps.iter().map(|s| (s.attempt, s.status)).collect();
    assert_eq!(
        attempts,
        [(1, StepRunStatus::Failed), (2, StepRunStatus::Succeeded)]
    );
}

#[tokio::test]
async fn a_disabled_member_fails_its_step() {
    let w = world().await;
    w.store.set_dot_enabled(&w.dev.id, false).await.unwrap();
    let flow = w
        .workflow("Fix", vec![agent("dev", "Fix it", &w.dev, "Fix.")])
        .await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, waits_on(WaitingOn::StepFailed)).await;
    assert_eq!(wr.error.as_deref(), Some("Fix it: dev is disabled"));
    assert_eq!(wr.spent_runs, 0);
}

#[tokio::test]
async fn stopping_cancels_its_runs_and_a_cancelled_run_fails_its_step() {
    let w = world().await;
    w.engine.set(&w.dev.spec.name, vec![Act::Hold]);
    let flow = w
        .workflow("Fix", vec![agent("dev", "Fix it", &w.dev, "Fix.")])
        .await;
    // A run cancelled on its own fails the step.
    let wr = w.start(&flow).await;
    let step = w.until(&wr.id, |wr| !wr.steps.is_empty()).await.steps[0]
        .id
        .clone();
    let run = loop {
        let runs = w.store.runs_of_step_run(&step).await.unwrap();
        if let Some(r) = runs.first().filter(|r| r.status == RunStatus::Running) {
            break r.clone();
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    };
    w.runner.cancel(&run.id).await.unwrap();
    let wr = w.until(&wr.id, waits_on(WaitingOn::StepFailed)).await;
    assert_eq!(wr.error.as_deref(), Some("Fix it: the run was cancelled"));

    // Stopping a workflow run cancels the run of its step.
    let second = w.start(&flow).await;
    let step = w.until(&second.id, |wr| !wr.steps.is_empty()).await.steps[0]
        .id
        .clone();
    let run = loop {
        let runs = w.store.runs_of_step_run(&step).await.unwrap();
        if let Some(r) = runs.first().filter(|r| r.status == RunStatus::Running) {
            break r.clone();
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    };
    let stopped = w.workflows.stop(&second.id).await.unwrap();
    assert_eq!(stopped.status, WorkflowRunStatus::Cancelled);
    assert_eq!(stopped.steps[0].status, StepRunStatus::Cancelled);
    common::wait_status(&w.store, &run.id, RunStatus::Cancelled, 10).await;
    assert!(matches!(
        w.workflows.stop(&second.id).await,
        Err(Error::Conflict(m)) if m.ends_with("is already cancelled")
    ));
}

#[tokio::test]
async fn a_question_holds_the_step_until_it_is_answered() {
    let w = world().await;
    w.engine.set(
        &w.dev.spec.name,
        vec![
            Act::Ask("Which icon set?"),
            Act::Write("icon.txt", "Used the light set."),
        ],
    );
    let flow = w
        .workflow(
            "Fix",
            vec![agent("dev", "Fix it", &w.dev, "Fix."), check("check")],
        )
        .await;
    let wr = w.start(&flow).await;
    let question = loop {
        let open = w
            .store
            .list_messages(Some(&w.dev.id))
            .await
            .unwrap()
            .into_iter()
            .find(|m| m.answered_at.is_none());
        if let Some(q) = open {
            break q;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    };
    // The run that asked has ended, and the step still waits.
    let asked = question.run_id.clone().unwrap();
    common::wait_status(&w.store, &asked, RunStatus::Succeeded, 10).await;
    w.workflows.pass().await.unwrap();
    let held = w.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(held.status, WorkflowRunStatus::Running);
    assert_eq!(held.steps[0].status, StepRunStatus::Running);
    // The answer continues the thread, and the step with it.
    let follow_ups = FollowUps::new(w.store.clone(), w.bus.clone(), Arc::new(SystemClock));
    let checkins = CheckIns::new(w.store.clone(), w.bus.clone(), follow_ups);
    let answer = checkins
        .reply(&w.runner, &question.id, "The light one.")
        .await
        .unwrap();
    assert!(answer.workflow_step_run_id.is_some());
    let wr = w.until(&wr.id, waits_on(WaitingOn::Check)).await;
    assert_eq!(
        wr.steps[0].handoff.as_ref().unwrap().summary,
        "Used the light set."
    );
    assert_eq!(wr.spent_runs, 2);
}

#[tokio::test]
async fn a_pass_after_a_restart_finds_an_interrupted_step() {
    let w = world().await;
    w.engine.set(&w.dev.spec.name, vec![Act::Hold]);
    let flow = w
        .workflow("Fix", vec![agent("dev", "Fix it", &w.dev, "Fix.")])
        .await;
    let wr = w.start(&flow).await;
    let step = w.until(&wr.id, |wr| !wr.steps.is_empty()).await.steps[0]
        .id
        .clone();
    loop {
        let runs = w.store.runs_of_step_run(&step).await.unwrap();
        if runs.first().is_some_and(|r| r.status == RunStatus::Running) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    // As Dots does at start after a crash: the running run is failed as interrupted, and a
    // new loop's first pass sees it.
    let interrupted = w.store.recover_interrupted().await.unwrap();
    assert_eq!(interrupted.len(), 1);
    let fresh = Workflows::new(
        w.store.clone(),
        new_bus(),
        w.runner.clone(),
        Arc::new(SystemClock),
    );
    fresh.pass().await.unwrap();
    let wr = w.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(wr.waiting_on, Some(WaitingOn::StepFailed));
    assert_eq!(wr.error.as_deref(), Some("Fix it: interrupted"));
}

#[tokio::test]
async fn what_a_start_refuses() {
    let w = world().await;
    let flow = w.standard().await;
    let mut req = StartRequest {
        workflow_id: flow.clone(),
        project_id: w.project.clone(),
        task: "  ".into(),
        trigger: WorkflowTrigger::Manual,
        origin_run_id: None,
        test: false,
    };
    assert!(matches!(
        w.workflows.start(req.clone()).await,
        Err(Error::Invalid(m)) if m == "the task must be 1 to 8,000 characters"
    ));
    req.task = "Fix it.".into();
    // A project of another team.
    let other = w
        .store
        .create_team(&TeamSpec {
            name: "Other".into(),
            description: String::new(),
            members: vec![],
        })
        .await
        .unwrap();
    let theirs = w
        .store
        .add_project(
            &other.id,
            &ProjectSpec {
                name: "Theirs".into(),
                repo_path: w.repo.to_string_lossy().to_string(),
                default_branch: None,
                remote: "origin".into(),
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        w.workflows.start(StartRequest { project_id: theirs.id, ..req.clone() }).await,
        Err(Error::Invalid(m)) if m == "Theirs is not a project of the workflow's team"
    ));
    // A workflow that is turned off.
    let mut spec = w.store.get_workflow(&flow).await.unwrap().spec;
    spec.enabled = false;
    w.store.update_workflow(&flow, &spec).await.unwrap();
    assert!(matches!(
        w.workflows.start(req.clone()).await,
        Err(Error::Conflict(m)) if m == "workflow Fix is turned off"
    ));
    spec.enabled = true;
    w.store.update_workflow(&flow, &spec).await.unwrap();
    // A workflow whose member left the team.
    w.store.delete_dot(&w.dev.id).await.unwrap();
    assert!(matches!(
        w.workflows.start(req.clone()).await,
        Err(Error::Invalid(m)) if m.starts_with("workflow Fix cannot start: step \"Fix it\":")
    ));
    assert!(w
        .store
        .list_workflow_runs(None, 10)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_team_with_runs_in_flight_is_deleted_by_stopping_them_first() {
    let w = world().await;
    w.engine.set(&w.dev.spec.name, vec![Act::Hold]);
    let flow = w
        .workflow("Fix", vec![agent("dev", "Fix it", &w.dev, "Fix.")])
        .await;
    let wr = w.start(&flow).await;
    let teams = dots_core::teams::Teams::new(w.store.clone(), w.bus.clone());
    assert!(matches!(
        teams.delete(&w.team).await,
        Err(Error::Conflict(m)) if m == "the team has workflow runs in flight; stop them first"
    ));
    w.workflows.delete_team(&w.team).await.unwrap();
    assert!(matches!(
        w.store.get_team(&w.team).await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        w.store.get_workflow_run(&wr.id).await,
        Err(Error::NotFound(_))
    ));
}

#[tokio::test]
async fn every_move_is_announced() {
    let w = world().await;
    let mut events = w.bus.subscribe();
    let flow = w
        .workflow(
            "Fix",
            vec![agent("dev", "Fix it", &w.dev, "Fix."), check("check")],
        )
        .await;
    w.start(&flow).await;
    // Every move until the check, in order; the event follows the store's write.
    let waiting = (WorkflowRunStatus::Waiting, Some("check".to_string()));
    let mut seen = Vec::new();
    while seen.last() != Some(&waiting) {
        let event = tokio::time::timeout(Duration::from_secs(60), events.recv())
            .await
            .expect("no event in time")
            .unwrap();
        if let RuntimeEvent::WorkflowRunUpdated { run } = event {
            seen.push((run.status, run.current_step));
        }
    }
    seen.dedup();
    assert_eq!(
        seen,
        [
            (WorkflowRunStatus::Running, None),
            (WorkflowRunStatus::Running, Some("dev".to_string())),
            waiting,
        ]
    );
    let _ = &w.dir;
}

#[tokio::test]
async fn a_pass_puts_a_run_cut_off_behind_a_failed_step_back_to_waiting() {
    let w = world().await;
    w.engine.set(&w.dev.spec.name, vec![Act::Finish("Fixed.")]);
    let flow = w
        .workflow("Fix", vec![agent("dev", "Fix it", &w.dev, "Fix.")])
        .await;
    w.store.set_dot_enabled(&w.dev.id, false).await.unwrap();
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, waits_on(WaitingOn::StepFailed)).await;
    // The move was cut off after the step was failed: the run is left running.
    w.store
        .set_workflow_run(
            &wr.id,
            &dots_core::model::WorkflowRunChange {
                status: Some(WorkflowRunStatus::Running),
                waiting_on: Some(None),
                error: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    w.workflows.pass().await.unwrap();
    let fixed = w.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(fixed.status, WorkflowRunStatus::Waiting);
    assert_eq!(fixed.waiting_on, Some(WaitingOn::StepFailed));
    assert_eq!(fixed.error.as_deref(), Some("Fix it: dev is disabled"));
    w.store.set_dot_enabled(&w.dev.id, true).await.unwrap();
    w.workflows.retry_step(&wr.id).await.unwrap();
    w.until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
}

#[tokio::test]
async fn a_pass_puts_a_run_cut_off_behind_a_waiting_check_back_to_waiting() {
    let w = world().await;
    let flow = w.workflow("Check", vec![check("check")]).await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, waits_on(WaitingOn::Check)).await;
    w.store
        .set_workflow_run(
            &wr.id,
            &dots_core::model::WorkflowRunChange {
                status: Some(WorkflowRunStatus::Running),
                waiting_on: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    w.workflows.pass().await.unwrap();
    let fixed = w.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(fixed.status, WorkflowRunStatus::Waiting);
    assert_eq!(fixed.waiting_on, Some(WaitingOn::Check));
    let done = w.workflows.approve_check(&wr.id, None).await.unwrap();
    assert_eq!(done.status, WorkflowRunStatus::Succeeded);
}
