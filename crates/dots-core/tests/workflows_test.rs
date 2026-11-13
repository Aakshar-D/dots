mod common;

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::{git_out, init_repo, spec, temp_store};
use dots_core::checkins::{CheckIns, Tool as CheckIn};
use dots_core::deploy::{DeployResult, DeployedBranch, Deployer, NOTHING_TO_SHIP};
use dots_core::engine::{Engine, EngineEvent, RunContext};
use dots_core::events::{new_bus, Bus, RuntimeEvent};
use dots_core::followups::FollowUps;
use dots_core::model::{
    Dot, EngineKind, FileStatus, FollowUpStatus, MemberHandoff, MemberSpec, NewRun, ProjectSpec,
    Run, RunStatus, SendBack, SentBy, StepRun, StepRunChange, StepRunStatus, TeamSpec, TriggerKind,
    WaitingOn, WorkflowRun, WorkflowRunChange, WorkflowRunFilter, WorkflowRunStatus, WorkflowSpec,
    WorkflowTrigger, WorkspaceMode,
};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::scheduler::{Clock, SystemClock};
use dots_core::store::Store;
use dots_core::team_tools::{DelegateTools, SendBackTarget, StartableMember, StepTools, Tool};
use dots_core::workflow::{
    BranchOn, BranchPath, Budget, Definition, Delegate, DeployMode, MergeMethod, Step, StepKind,
    Trigger,
};
use dots_core::workflows::{
    CheckTip, CheckView, ShownBranch, StartRequest, Workflows, ENDED_STEP_THREAD,
    NO_DEFAULT_BRANCH, STEP_CLOSED,
};
use dots_core::Error;
use serde_json::json;
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
    /// Reports this usage (in, out, cache read), then finishes.
    Use(u64, u64, u64),
    /// Waits until the test opens the gate, then finishes.
    Gate,
    /// Writes a file into its workspace, asks an approval, and finishes with this summary:
    /// the run parks on the approval.
    Park(&'static str, &'static str),
    /// Schedules a follow-up in an hour, then finishes with this summary.
    Schedule(&'static str),
    /// Writes a file into its workspace and tags the commit its branch is on with the
    /// branch's own name, then finishes with this summary.
    WriteAndTag(&'static str, &'static str),
    /// Finishes with this summary, made at run time.
    Say(String),
    /// Calls `start_member` with each (member, task), as the run's model would, keeping the
    /// answers (`Team::answers`); then does the act given.
    Start(Vec<(&'static str, &'static str)>, Box<Act>),
    /// Waits for a permit of `Team::held`, then writes a file into its workspace and finishes
    /// with this summary.
    Await(&'static str, &'static str),
    /// Calls `send_back` with (step name, note), as the run's model would, keeping the answer
    /// (`Team::answers`); then does the act given.
    SendBack(&'static str, &'static str, Box<Act>),
    /// Starts no session: writes the file given, if any, into its workspace and finishes with
    /// this summary, so the run cannot be resumed.
    NoSession(Option<&'static str>, &'static str),
    /// Merges into its branch (`git merge --no-ff`) the branch of the first member its step
    /// run started, in the order started, whose branch still exists; then writes a file into
    /// its workspace and finishes with this summary.
    MergeFirst(&'static str, &'static str),
}

/// Whether the branch `branch` exists in the repository of `dir`.
fn branch_in(dir: &Path, branch: &str) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}"))
        .output()
        .unwrap()
        .status
        .success()
}

/// Plays each dot's acts in order, one per run (the last one again once they run out), and
/// keeps each run's prompt and workflow tools.
struct Team {
    store: Store,
    acts: Mutex<HashMap<String, VecDeque<Act>>>,
    prompts: Mutex<Vec<(String, String)>>,
    step_tools: Mutex<Vec<(String, Option<StepTools>)>>,
    gate: Arc<tokio::sync::Notify>,
    /// What `Act::Await` waits on: one permit per run.
    held: Arc<tokio::sync::Semaphore>,
    /// The workflows the runs' `start_member` calls go to.
    workflows: std::sync::OnceLock<std::sync::Weak<Workflows>>,
    /// The answers of the `start_member` calls, in the order made.
    answers: Arc<Mutex<Vec<Result<String, String>>>>,
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

    fn step_tools_of(&self, dot: &str) -> Vec<Option<StepTools>> {
        self.step_tools
            .lock()
            .unwrap()
            .iter()
            .filter(|(d, _)| d == dot)
            .map(|(_, t)| t.clone())
            .collect()
    }

    fn answers(&self) -> Vec<Result<String, String>> {
        self.answers.lock().unwrap().clone()
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
        self.step_tools
            .lock()
            .unwrap()
            .push((name.clone(), ctx.tools.step().cloned()));
        let mut act = self.act(&name);
        let store = self.store.clone();
        let gate = self.gate.clone();
        let held = self.held.clone();
        let workflows = self.workflows.get().cloned();
        let answers = self.answers.clone();
        let (tx, rx) = mpsc::channel(8);
        tokio::spawn(async move {
            if !matches!(act, Act::NoSession(..)) {
                // A resumed run goes on in the session it was given.
                let session = ctx
                    .session_id
                    .clone()
                    .unwrap_or_else(|| format!("session-{}", ctx.run_id));
                let _ = tx
                    .send(EngineEvent::SessionStarted {
                        session_id: session,
                    })
                    .await;
            }
            if let Act::SendBack(to, note, then) = act {
                let workflows = workflows.clone().and_then(|w| w.upgrade());
                let workflows = workflows.expect("the workflows");
                let input = json!({"to_step": to, "note": note});
                let answer = match ctx.tools.step() {
                    Some(step) => workflows.call_send_back(&ctx.run_id, step, &input).await,
                    None => Err("this run has no step tools".to_string()),
                };
                answers.lock().unwrap().push(answer);
                act = *then;
            }
            if let Act::Start(calls, then) = act {
                let workflows = workflows.and_then(|w| w.upgrade()).expect("the workflows");
                for (member, task) in calls {
                    let input = json!({"member": member, "task": task});
                    let answer = match ctx.tools.step() {
                        Some(step) => workflows.call_start_member(&ctx.run_id, step, &input).await,
                        None => Err("this run has no step tools".to_string()),
                    };
                    answers.lock().unwrap().push(answer);
                }
                act = *then;
            }
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
                Act::Use(tokens_in, tokens_out, cache_read) => {
                    let usage = EngineEvent::Usage {
                        tokens_in,
                        tokens_out,
                        cache_read,
                        cost_usd: None,
                    };
                    let _ = tx.send(usage).await;
                    EngineEvent::Finished {
                        summary: "used".into(),
                    }
                }
                Act::Gate => {
                    gate.notified().await;
                    EngineEvent::Finished {
                        summary: "through".into(),
                    }
                }
                Act::Park(file, s) => {
                    std::fs::write(ctx.workspace.join(file), s).unwrap();
                    store
                        .create_approval(&ctx.run_id, "Bash", &json!({"command": "git push"}))
                        .await
                        .unwrap();
                    EngineEvent::Finished { summary: s.into() }
                }
                Act::WriteAndTag(file, s) => {
                    std::fs::write(ctx.workspace.join(file), s).unwrap();
                    let branch = format!("dots/{name}/{}", ctx.run_id);
                    git_out(&ctx.workspace, &["tag", &branch]);
                    EngineEvent::Finished { summary: s.into() }
                }
                Act::Say(s) => EngineEvent::Finished { summary: s },
                Act::Start(..) | Act::SendBack(..) => unreachable!("played above"),
                Act::NoSession(file, s) => {
                    if let Some(file) = file {
                        std::fs::write(ctx.workspace.join(file), s).unwrap();
                    }
                    EngineEvent::Finished { summary: s.into() }
                }
                Act::MergeFirst(file, s) => {
                    let run = store.get_run(&ctx.run_id).await.unwrap();
                    let step_run = run.workflow_step_run_id.expect("a step's run");
                    let mut first = None;
                    for m in store.step_members(&step_run).await.unwrap() {
                        let branch = store.get_run(&m.run_id).await.unwrap().branch;
                        if let Some(b) = branch.filter(|b| branch_in(&ctx.workspace, b)) {
                            first = Some(b);
                            break;
                        }
                    }
                    let first = first.expect("a member's branch");
                    let args = ["merge", "--no-ff", "-q", "-m", "Merge a part.", &first];
                    git_out(&ctx.workspace, &args);
                    std::fs::write(ctx.workspace.join(file), s).unwrap();
                    EngineEvent::Finished { summary: s.into() }
                }
                Act::Await(file, s) => {
                    held.acquire().await.unwrap().forget();
                    std::fs::write(ctx.workspace.join(file), s).unwrap();
                    EngineEvent::Finished { summary: s.into() }
                }
                Act::Schedule(s) => {
                    let now = chrono::Utc::now();
                    let due = now + chrono::TimeDelta::hours(1);
                    store
                        .add_follow_up(&ctx.dot.id, &ctx.run_id, due, "Look again.", 10, now)
                        .await
                        .unwrap()
                        .unwrap();
                    EngineEvent::Finished { summary: s.into() }
                }
            };
            let _ = tx.send(end).await;
        });
        Ok(rx)
    }
}

struct World {
    dir: TempDir,
    /// The budget of the workflows `World::workflow` makes.
    budget: Budget,
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
    /// Stops the workflows' loop, as Dots stopping does.
    stop_loop: CancellationToken,
}

async fn worktree_dot(store: &Store, dir: &Path, name: &str) -> Dot {
    let own = dir.join(format!("{name}-own"));
    init_repo(&own);
    let mut s = spec(name);
    s.workdir = own.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Worktree;
    store.create_dot(&s, None, "default").await.unwrap()
}

async fn world() -> World {
    world_with(Arc::new(SystemClock), Budget::default()).await
}

async fn world_with(clock: Arc<dyn Clock>, budget: Budget) -> World {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let engine = Arc::new(Team {
        store: store.clone(),
        acts: Mutex::new(HashMap::new()),
        prompts: Mutex::new(Vec::new()),
        step_tools: Mutex::new(Vec::new()),
        gate: Arc::new(tokio::sync::Notify::new()),
        held: Arc::new(tokio::sync::Semaphore::new(0)),
        workflows: std::sync::OnceLock::new(),
        answers: Arc::new(Mutex::new(Vec::new())),
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
            quiet_unit: Duration::from_secs(1),
        },
    );
    runner.spawn_dispatcher(CancellationToken::new());
    let workflows = Workflows::new(store.clone(), bus.clone(), runner.clone(), clock);
    let _ = engine.workflows.set(Arc::downgrade(&workflows));
    let stop_loop = CancellationToken::new();
    workflows.spawn(stop_loop.clone());
    let repo = dir.path().join("project");
    init_repo(&repo);
    let helper = worktree_dot(&store, dir.path(), "helper").await;
    let dev = worktree_dot(&store, dir.path(), "dev").await;
    let team = store
        .create_team(&TeamSpec {
            org_id: None,
            workspace_id: "default".into(),
            name: "Sage management".into(),
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
                name: "Sage".into(),
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
        budget,
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
        stop_loop,
    }
}

fn agent(id: &str, name: &str, member: &Dot, job: &str) -> Step {
    Step {
        id: id.into(),
        name: name.into(),
        kind: StepKind::Agent {
            member: member.id.clone(),
            job: job.into(),
            can_send_back_to: vec![],
            outcomes: vec![],
            delegate: None,
        },
    }
}

fn check(id: &str) -> Step {
    Step {
        id: id.into(),
        name: "Your check".into(),
        kind: StepKind::Check { send_back_to: None },
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
                        budget: self.budget,
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
    /// Moves `clock` to `to` in steps of 30 s (the loop's tick), with a pass after each, as the
    /// loop would meanwhile.
    async fn run_clock(&self, clock: &common::clock::FakeClock, to: chrono::DateTime<chrono::Utc>) {
        let mut at = clock.now();
        while at < to {
            at = (at + chrono::TimeDelta::seconds(30)).min(to);
            clock.set(at);
            self.workflows.pass().await.unwrap();
        }
    }

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

impl World {
    /// Approves the check the workflow run `id` waits on as the check page does: with the tips
    /// its check shows now.
    async fn approve(&self, id: &str, note: Option<&str>) -> dots_core::Result<WorkflowRun> {
        let view = self.workflows.check(id).await?;
        self.workflows
            .approve_check(id, note, &view.step_run_id, &view.tips())
            .await
    }

    /// Sends the work back from the check the workflow run `id` waits on as the check page
    /// does: naming the step run its check shows now (none when it waits on no check).
    async fn send_back(&self, id: &str, note: &str) -> dots_core::Result<WorkflowRun> {
        let step_run = match self.workflows.check(id).await {
            Ok(view) => view.step_run_id,
            Err(_) => String::new(),
        };
        self.workflows.send_back_check(id, note, &step_run).await
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
        "You are the dev agent of the team \"Sage management\", doing one step of the workflow \"Fix\".",
        "- Step: Fix it (2 of 3)",
        "### Task\n\nThe tray icon is invisible in dark mode.",
        "### Your job in this step\n\nFix the bug.",
        "### Earlier steps\n\n#### Write the task\n\nTask: make the icon visible.",
    ] {
        assert!(prompt.contains(part), "missing {part:?} in:\n{prompt}");
    }

    let done = w.approve(&wr.id, Some("looks good")).await.unwrap();
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
        w.workflows.approve_check(&wr.id, None, "", &[]).await,
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
async fn a_deploy_after_a_check_that_showed_no_branch_fails_unless_it_only_lists_branches() {
    let w = world().await;
    w.shipping_to_origin("ok");
    // On no branch, so a pull request's base could not be found either: nothing to ship is
    // what the step says.
    git_out(&w.repo, &["checkout", "-q", "--detach"]);
    let modes = [
        DeployMode::Push,
        DeployMode::Pr,
        DeployMode::PrMerge {
            merge_method: MergeMethod::Squash,
        },
    ];
    for (n, mode) in modes.into_iter().enumerate() {
        let flow = w
            .workflow(&format!("Deploy {n}"), vec![check("check"), ship(mode)])
            .await;
        let wr = w.start(&flow).await;
        let wr = w.until(&wr.id, waits_on(WaitingOn::Check)).await;
        w.approve(&wr.id, None).await.unwrap();
        let wr = w.until(&wr.id, waits_on(WaitingOn::StepFailed)).await;
        assert_eq!(
            wr.steps[0].decision.as_ref().unwrap()["branches"],
            json!([])
        );
        assert_eq!(
            wr.error.as_deref(),
            Some("Ship: Nothing to ship: the check approved no branch.")
        );
        let ship = newest(&wr, "ship");
        assert_eq!(ship.status, StepRunStatus::Failed);
        assert_eq!(
            ship.deploy_result,
            Some(DeployResult {
                branches: vec![],
                ready: false,
                error: Some(NOTHING_TO_SHIP.into()),
            })
        );
        w.workflows.stop(&wr.id).await.unwrap();
    }
    assert!(w.gh_calls().is_empty());
    // Mode `branch` lists the branches it was given: none, and it succeeds.
    let flow = w
        .workflow(
            "Deploy branch",
            vec![check("check"), ship(DeployMode::Branch)],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, waits_on(WaitingOn::Check)).await;
    w.approve(&wr.id, None).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(
        newest(&wr, "ship").deploy_result,
        Some(DeployResult {
            branches: vec![],
            ready: true,
            error: None,
        })
    );
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
    common::wait_status(&w.store, &run.id, RunStatus::Cancelled, 60).await;
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
            org_id: None,
            workspace_id: "default".into(),
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
async fn a_test_run_starts_while_its_workflow_is_off_and_run_now_does_not() {
    let w = world().await;
    let flow = w.standard().await;
    let mut spec = w.store.get_workflow(&flow).await.unwrap().spec;
    spec.enabled = false;
    w.store.update_workflow(&flow, &spec).await.unwrap();
    let run_now = StartRequest {
        workflow_id: flow.clone(),
        project_id: w.project.clone(),
        task: "The tray icon is invisible in dark mode.".into(),
        trigger: WorkflowTrigger::Manual,
        origin_run_id: None,
        test: false,
    };
    // Run now (a manual start that is not a test) is still refused, with the same words.
    assert!(matches!(
        w.workflows.start(run_now.clone()).await,
        Err(Error::Conflict(m)) if m == "workflow Fix is turned off"
    ));
    // Only the manual trigger may test an off workflow: a start from a trigger `enabled` arms
    // is refused even when it asks for a test.
    for trigger in [
        WorkflowTrigger::Chat,
        WorkflowTrigger::Schedule,
        WorkflowTrigger::Webhook,
    ] {
        assert!(matches!(
            w.workflows
                .start(StartRequest { trigger, test: true, ..run_now.clone() })
                .await,
            Err(Error::Conflict(m)) if m == "workflow Fix is turned off"
        ));
    }
    assert!(w
        .store
        .list_workflow_runs(None, 10)
        .await
        .unwrap()
        .is_empty());
    // A Test run starts, is flagged `test`, and goes on to the check as any run does.
    let started = w
        .workflows
        .start(StartRequest {
            test: true,
            ..run_now.clone()
        })
        .await
        .unwrap();
    assert!(started.test);
    assert_eq!(started.trigger, WorkflowTrigger::Manual);
    assert_eq!(started.status, WorkflowRunStatus::Running);
    let wr = w.until(&started.id, waits_on(WaitingOn::Check)).await;
    assert!(wr.test);
    // The workflow is still off, and Run now is still refused.
    assert!(!w.store.get_workflow(&flow).await.unwrap().spec.enabled);
    assert!(matches!(
        w.workflows.start(run_now).await,
        Err(Error::Conflict(m)) if m == "workflow Fix is turned off"
    ));
    let runs = w.store.list_workflow_runs(None, 10).await.unwrap();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].id, started.id);
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
async fn the_runs_limit_holds_the_next_step_until_more_budget_is_added() {
    let w = world_with(
        Arc::new(SystemClock),
        Budget {
            max_runs: 1,
            ..Budget::default()
        },
    )
    .await;
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
    let wr = w.until(&wr.id, waits_on(WaitingOn::Budget)).await;
    assert_eq!(wr.error.as_deref(), Some("Budget reached: runs, 1 of 1"));
    assert_eq!(wr.current_step.as_deref(), Some("one"));
    assert_eq!(wr.steps.len(), 1);
    assert!(matches!(
        w.workflows.retry_step(&wr.id).await,
        Err(Error::Conflict(_))
    ));
    let more = w.workflows.add_budget(&wr.id).await.unwrap();
    assert_eq!(more.budget.max_runs, 11);
    assert_eq!(more.error, None);
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(wr.spent_runs, 2);
    assert!(matches!(
        w.workflows.add_budget(&wr.id).await,
        Err(Error::Conflict(m)) if m.ends_with("is not waiting for budget")
    ));
}

#[tokio::test]
async fn the_token_limit_counts_new_input_and_output_but_not_cache_reads() {
    let w = world_with(
        Arc::new(SystemClock),
        Budget {
            max_tokens: 1_000,
            ..Budget::default()
        },
    )
    .await;
    // 5,000 in, of which 4,500 from the cache, and 400 out: 900 counted, under the limit.
    w.engine.set(
        &w.dev.spec.name,
        vec![
            Act::Use(5_000, 400, 4_500),
            Act::Use(200, 0, 0),
            Act::Finish("never"),
        ],
    );
    let flow = w
        .workflow(
            "Three",
            vec![
                agent("one", "First", &w.dev, "Start."),
                agent("two", "Second", &w.dev, "Go on."),
                agent("three", "Third", &w.dev, "End."),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, waits_on(WaitingOn::Budget)).await;
    // The second step ran (900 < 1,000) and passed the limit; the third waits.
    assert_eq!((wr.spent_tokens, wr.spent_cache_read), (1_100, 4_500));
    assert_eq!(
        wr.error.as_deref(),
        Some("Budget reached: tokens, 1100 of 1000")
    );
    assert_eq!(wr.current_step.as_deref(), Some("two"));
    let more = w.workflows.add_budget(&wr.id).await.unwrap();
    assert_eq!(more.budget.max_tokens, 1_501_000);
}

#[tokio::test]
async fn the_clock_counts_only_while_nobody_waits_on_the_person() {
    let start = chrono::Utc::now();
    let clock = common::clock::FakeClock::new(start);
    let w = world_with(
        clock.clone(),
        Budget {
            max_minutes: 10,
            ..Budget::default()
        },
    )
    .await;
    w.engine
        .set(&w.dev.spec.name, vec![Act::Gate, Act::Finish("Fixed.")]);
    let flow = w
        .workflow(
            "Fix",
            vec![
                agent("one", "First", &w.dev, "Start."),
                check("check"),
                agent("two", "Second", &w.dev, "Go on."),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    w.until(&wr.id, |wr| !wr.steps.is_empty()).await;
    // Four minutes of work count.
    w.run_clock(&clock, start + chrono::TimeDelta::minutes(4))
        .await;
    assert_eq!(
        w.store.get_workflow_run(&wr.id).await.unwrap().active_ms,
        240_000
    );
    w.engine.gate.notify_one();
    let wr = w.until(&wr.id, waits_on(WaitingOn::Check)).await;
    // An hour at the check counts nothing.
    clock.set(start + chrono::TimeDelta::minutes(64));
    w.workflows.pass().await.unwrap();
    assert_eq!(
        w.store.get_workflow_run(&wr.id).await.unwrap().active_ms,
        240_000
    );
    w.approve(&wr.id, None).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(wr.active_ms, 240_000);

    // Past the minutes limit, the next step waits.
    // The engine repeats its last act: set the gate again for the second run's step.
    w.engine
        .set(&w.dev.spec.name, vec![Act::Gate, Act::Finish("Fixed.")]);
    let second = w.start(&flow).await;
    w.until(&second.id, |wr| !wr.steps.is_empty()).await;
    w.run_clock(&clock, start + chrono::TimeDelta::minutes(80))
        .await;
    w.engine.gate.notify_one();
    let second = w.until(&second.id, waits_on(WaitingOn::Check)).await;
    w.approve(&second.id, None).await.unwrap();
    let second = w.until(&second.id, waits_on(WaitingOn::Budget)).await;
    assert_eq!(
        second.error.as_deref(),
        Some("Budget reached: minutes, 16 of 10")
    );
}

#[tokio::test]
async fn a_gap_in_the_clock_counts_at_most_a_minute() {
    let start = chrono::Utc::now();
    let clock = common::clock::FakeClock::new(start);
    let w = world_with(clock.clone(), Budget::default()).await;
    w.engine.set(&w.dev.spec.name, vec![Act::Hold]);
    let flow = w
        .workflow("Hold", vec![agent("one", "First", &w.dev, "Start.")])
        .await;
    let wr = w.start(&flow).await;
    w.until(&wr.id, |wr| !wr.steps.is_empty()).await;
    // The app was closed, or the machine slept, for three hours.
    clock.set(start + chrono::TimeDelta::hours(3));
    w.workflows.pass().await.unwrap();
    let wr = w.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(wr.active_ms, 60_000);
    w.workflows.stop(&wr.id).await.unwrap();
}

#[tokio::test]
async fn add_budget_after_a_held_retry_starts_the_failed_step() {
    let w = world_with(
        Arc::new(SystemClock),
        Budget {
            max_runs: 1,
            ..Budget::default()
        },
    )
    .await;
    w.engine.set(
        &w.dev.spec.name,
        vec![Act::Fail("compiler exploded"), Act::Finish("Fixed.")],
    );
    let flow = w
        .workflow("Fix", vec![agent("dev", "Fix it", &w.dev, "Fix.")])
        .await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, waits_on(WaitingOn::StepFailed)).await;
    // The limit holds the retry.
    w.workflows.retry_step(&wr.id).await.unwrap();
    let wr = w.until(&wr.id, waits_on(WaitingOn::Budget)).await;
    assert_eq!(wr.spent_runs, 1);
    w.workflows.add_budget(&wr.id).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(wr.spent_runs, 2);
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
    let done = w.approve(&wr.id, None).await.unwrap();
    assert_eq!(done.status, WorkflowRunStatus::Succeeded);
}

impl World {
    /// The first run of the workflow run's first step once it has parked on its approval and
    /// the runner has let it go, with that approval denied without a note.
    async fn parked_and_denied(&self, id: &str) -> (dots_core::model::Run, String) {
        let step = self.until(id, |wr| !wr.steps.is_empty()).await.steps[0]
            .id
            .clone();
        let run = loop {
            let runs = self.store.runs_of_step_run(&step).await.unwrap();
            if let Some(r) = runs.first().filter(|r| {
                r.status == RunStatus::AwaitingApproval && !self.runner.is_active(&r.id)
            }) {
                break r.clone();
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        };
        let approval = self
            .store
            .approvals_for_run(&run.id)
            .await
            .unwrap()
            .remove(0);
        self.store
            .decide_approval_with_parked(&approval.id, false, None, true)
            .await
            .unwrap();
        (run, approval.id)
    }

    /// Asserts that the first step handed on a branch with `park.txt` committed, and that the
    /// second step's worktree started from it.
    async fn second_step_has_the_parked_work(&self, wr: &WorkflowRun) {
        let first = &wr.steps[0].handoff.as_ref().unwrap().branches[0].branch;
        assert_eq!(
            git_out(&self.repo, &["show", &format!("{first}:park.txt")]),
            "Parked."
        );
        let second = self.store.runs_of_step_run(&wr.steps[1].id).await.unwrap();
        let base = second[0].base_commit.clone().unwrap();
        assert_eq!(
            git_out(&self.repo, &["show", &format!("{base}:park.txt")]),
            "Parked."
        );
    }

    async fn parking_flow(&self) -> String {
        self.engine.set(
            &self.dev.spec.name,
            vec![Act::Park("park.txt", "Parked."), Act::Finish("Went on.")],
        );
        self.workflow(
            "Park",
            vec![
                agent("one", "First", &self.dev, "Start."),
                agent("two", "Second", &self.dev, "Go on."),
            ],
        )
        .await
    }
}

#[tokio::test]
async fn a_parked_step_run_denied_without_a_note_hands_on_its_work_committed() {
    let w = world().await;
    let flow = w.parking_flow().await;
    let wr = w.start(&flow).await;
    let (run, _) = w.parked_and_denied(&wr.id).await;
    // No resume: the run ends here, and its work is committed before it reads as ended.
    assert!(w.runner.maybe_resume(&run.id).await.unwrap().is_none());
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    w.second_step_has_the_parked_work(&wr).await;
}

#[tokio::test]
async fn the_hand_off_commits_what_a_run_left_uncommitted() {
    let w = world().await;
    let flow = w.parking_flow().await;
    let wr = w.start(&flow).await;
    let (run, approval) = w.parked_and_denied(&wr.id).await;
    // As if Dots stopped between closing the run and committing its work: the run reads as
    // ended, its work is still uncommitted in its worktree.
    w.store
        .resume_parked(&run.id, &[], &[approval], &[], None)
        .await
        .unwrap();
    w.workflows.pass().await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    w.second_step_has_the_parked_work(&wr).await;
}

#[tokio::test]
async fn stopping_dismisses_its_steps_questions_and_cancels_their_follow_ups() {
    let w = world().await;
    w.engine
        .set(&w.dev.spec.name, vec![Act::Ask("Which icon set?")]);
    let flow = w
        .workflow("Fix", vec![agent("dev", "Fix it", &w.dev, "Fix.")])
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
    let asked = question.run_id.clone().unwrap();
    common::wait_status(&w.store, &asked, RunStatus::Succeeded, 60).await;
    let now = chrono::Utc::now();
    let follow_up = w
        .store
        .add_follow_up(
            &w.dev.id,
            &asked,
            now + chrono::TimeDelta::hours(1),
            "Look again.",
            10,
            now,
        )
        .await
        .unwrap()
        .unwrap();
    let stopped = w.workflows.stop(&wr.id).await.unwrap();
    assert_eq!(stopped.status, WorkflowRunStatus::Cancelled);
    let question = w.store.get_message(&question.id).await.unwrap();
    assert!(question.dismissed_at.is_some());
    assert_eq!(
        w.store.get_follow_up(&follow_up.id).await.unwrap().status,
        FollowUpStatus::Cancelled
    );
    // Answering the question starts nothing.
    let follow_ups = FollowUps::new(w.store.clone(), w.bus.clone(), Arc::new(SystemClock));
    let checkins = CheckIns::new(w.store.clone(), w.bus.clone(), follow_ups);
    assert!(checkins
        .reply(&w.runner, &question.id, "The light one.")
        .await
        .is_err());
    assert_eq!(
        w.store.list_runs(Some(&w.dev.id), 10).await.unwrap().len(),
        1
    );
}

#[tokio::test]
async fn a_step_s_run_asks_and_schedules_follow_ups_only_while_its_step_is_open() {
    let w = world().await;
    w.engine.set(&w.dev.spec.name, vec![Act::Hold]);
    let flow = w
        .workflow("Fix", vec![agent("dev", "Fix it", &w.dev, "Fix.")])
        .await;
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
    let follow_ups = FollowUps::new(w.store.clone(), w.bus.clone(), Arc::new(SystemClock));
    let checkins = CheckIns::new(w.store.clone(), w.bus.clone(), follow_ups.clone());
    checkins.set_workflows(&w.workflows);
    let ask = json!({"question": "Which icon set?"});
    let later = json!({"note": "Look again.", "in": "1h"});
    // While the step is open, its run schedules a follow-up as any run does.
    let scheduled = checkins.call(&run.id, CheckIn::FollowUp, &later).await;
    assert!(scheduled.unwrap().starts_with("Follow-up "));
    w.workflows.stop(&wr.id).await.unwrap();
    assert!(w
        .store
        .list_pending_follow_ups(Some(&w.dev.id))
        .await
        .unwrap()
        .is_empty());
    // A call that comes after the Stop closed the step, while the engine has not yet ended the
    // run, is refused and records nothing; with and without the workflow runs.
    let bare = CheckIns::new(w.store.clone(), w.bus.clone(), follow_ups);
    for checkins in [&checkins, &bare] {
        for (tool, input) in [(CheckIn::Ask, &ask), (CheckIn::FollowUp, &later)] {
            assert_eq!(
                checkins.call(&run.id, tool, input).await,
                Err(STEP_CLOSED.to_string())
            );
        }
    }
    let messages = w.store.list_messages(Some(&w.dev.id)).await.unwrap();
    assert!(messages
        .iter()
        .all(|m| m.run_id.as_deref() != Some(run.id.as_str())));
    assert!(w
        .store
        .list_pending_follow_ups(Some(&w.dev.id))
        .await
        .unwrap()
        .is_empty());
}

/// The window a Stop can hit: the step's run has turned `Running`, and the step is closed
/// before the runner reads it and before the Stop's cancel reaches the run. The queue is held
/// so the run is claimed only once its step is closed as a Stop closes it; nothing cancels the
/// run itself.
#[tokio::test]
async fn a_run_whose_step_was_stopped_before_its_engine_started_ends_cancelled() {
    let w = world().await;
    w.engine.set(&w.dev.spec.name, vec![Act::Hold]);
    let flow = w
        .workflow("Fix", vec![agent("dev", "Fix it", &w.dev, "Fix.")])
        .await;
    w.runner.set_held(true).await;
    let wr = w.start(&flow).await;
    let step = w.until(&wr.id, |wr| !wr.steps.is_empty()).await.steps[0]
        .id
        .clone();
    let run = loop {
        let runs = w.store.runs_of_step_run(&step).await.unwrap();
        if let Some(r) = runs.first() {
            break r.clone();
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    };
    assert_eq!(run.status, RunStatus::Queued);
    w.store
        .set_step_run(
            &step,
            &StepRunChange {
                status: Some(StepRunStatus::Cancelled),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    w.store
        .set_workflow_run(
            &wr.id,
            &WorkflowRunChange {
                status: Some(WorkflowRunStatus::Cancelled),
                waiting_on: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    w.runner.set_held(false).await;
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let ended = loop {
        let r = w.store.get_run(&run.id).await.unwrap();
        if r.status.is_terminal() {
            break r;
        }
        assert!(std::time::Instant::now() < deadline, "the run never ended");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(ended.status, RunStatus::Cancelled);
    assert_eq!(ended.error, None);
    // The engine never started.
    assert!(w.engine.prompts_of(&w.dev.spec.name).is_empty());
}

#[tokio::test]
async fn a_follow_up_scheduled_in_a_step_is_cancelled_when_the_step_ends() {
    let w = world().await;
    w.engine
        .set(&w.dev.spec.name, vec![Act::Schedule("Fixed.")]);
    let flow = w
        .workflow("Fix", vec![agent("dev", "Fix it", &w.dev, "Fix.")])
        .await;
    let wr = w.start(&flow).await;
    w.until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert!(w
        .store
        .list_pending_follow_ups(Some(&w.dev.id))
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn a_reply_in_an_ended_steps_thread_gets_no_step_and_leaves_its_worktree() {
    let w = world().await;
    w.engine
        .set(&w.dev.spec.name, vec![Act::Write("fix.txt", "Fixed.")]);
    let flow = w
        .workflow("Fix", vec![agent("dev", "Fix it", &w.dev, "Fix.")])
        .await;
    let wr = w.start(&flow).await;
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    let run = w
        .store
        .runs_of_step_run(&wr.steps[0].id)
        .await
        .unwrap()
        .remove(0);
    let worktree = PathBuf::from(run.workspace_path.clone().unwrap());
    let branch = run.branch.clone().unwrap();
    let tip = git_out(&w.repo, &["rev-parse", &branch]);
    // A reply as `Runtime::chat` makes it, which refuses it first.
    assert!(w.store.in_ended_step_thread(&run).await.unwrap());
    let mut new = NewRun::new(&w.dev.id, TriggerKind::Chat);
    new.payload = Some(json!({"message": "And the tooltip?"}));
    new.parent_run_id = Some(run.id.clone());
    new.root_run_id = Some(run.root_run_id.clone());
    new.session_id = run.session_id.clone();
    new.workspace_path = run.workspace_path.clone();
    new.branch = run.branch.clone();
    new.base_commit = run.base_commit.clone();
    let reply = w.runner.enqueue(new).await.unwrap();
    assert_eq!(reply.workflow_step_run_id, None);
    let reply = common::wait_status(&w.store, &reply.id, RunStatus::Failed, 60).await;
    assert_eq!(reply.error.as_deref(), Some(ENDED_STEP_THREAD));
    // The step's worktree and branch are as the step left them, and the dot never ran.
    assert_eq!(git_out(&w.repo, &["rev-parse", &branch]), tip);
    assert!(worktree.is_dir());
    assert_eq!(git_out(&worktree, &["status", "--porcelain"]), "");
    assert_eq!(w.engine.prompts_of("dev").len(), 1);
    assert_eq!(
        w.store.get_workflow_run(&wr.id).await.unwrap().spent_runs,
        1
    );
}

#[tokio::test]
async fn a_tag_named_like_the_earlier_steps_branch_does_not_move_the_next_steps_start() {
    let w = world().await;
    w.engine.set(
        &w.dev.spec.name,
        vec![
            Act::WriteAndTag("one.txt", "First."),
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
    let tip = git_out(&w.repo, &["rev-parse", &format!("refs/heads/{first}")]);
    let second = w.store.runs_of_step_run(&wr.steps[1].id).await.unwrap();
    assert_eq!(second[0].base_commit.as_deref(), Some(tip.as_str()));
}

#[tokio::test]
async fn a_step_whose_member_was_deleted_fails_instead_of_queueing_again() {
    let w = world().await;
    w.engine
        .set(&w.dev.spec.name, vec![Act::Ask("Which icon set?")]);
    let flow = w
        .workflow("Fix", vec![agent("dev", "Fix it", &w.dev, "Fix.")])
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
    common::wait_status(
        &w.store,
        question.run_id.as_deref().unwrap(),
        RunStatus::Succeeded,
        60,
    )
    .await;
    // While the step waits on the question, the person deletes its member: the step's runs and
    // the question go with it.
    w.store.delete_dot_if_idle(&w.dev.id).await.unwrap();
    w.workflows.pass().await.unwrap();
    let wr = w.until(&wr.id, waits_on(WaitingOn::StepFailed)).await;
    assert_eq!(
        wr.error.as_deref(),
        Some("Fix it: its member is no longer a sol")
    );
    assert_eq!(wr.steps[0].status, StepRunStatus::Failed);
}

#[tokio::test]
async fn a_long_summary_is_cut_to_the_hand_off_limit_mark_included() {
    let w = world().await;
    let long = "\u{e9}".repeat(4_100);
    w.engine.set(&w.dev.spec.name, vec![Act::Say(long.clone())]);
    let flow = w
        .workflow("Fix", vec![agent("dev", "Fix it", &w.dev, "Fix.")])
        .await;
    let wr = w.start(&flow).await;
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    let summary = wr.steps[0].handoff.as_ref().unwrap().summary.clone();
    assert_eq!(summary.chars().count(), 4_000);
    assert!(summary.ends_with(" [cut]"), "{summary}");
    assert!(long.starts_with(summary.trim_end_matches(" [cut]")));
}

/// A branch step with `paths` (label, steps) and `otherwise`.
fn branch(id: &str, on: BranchOn, paths: Vec<(&str, Vec<Step>)>, otherwise: Vec<Step>) -> Step {
    Step {
        id: id.into(),
        name: format!("Branch {id}"),
        kind: StepKind::Branch {
            on,
            paths: paths
                .into_iter()
                .map(|(label, steps)| BranchPath {
                    label: label.into(),
                    steps,
                })
                .collect(),
            otherwise,
        },
    }
}

/// An agent step that may report `outcomes`.
fn reporting(id: &str, name: &str, member: &Dot, outcomes: &[&str]) -> Step {
    let mut step = agent(id, name, member, "Look into it.");
    if let StepKind::Agent { outcomes: o, .. } = &mut step.kind {
        *o = outcomes.iter().map(|s| s.to_string()).collect();
    }
    step
}

/// The step ids of the workflow run's step runs, oldest first.
fn ran(wr: &WorkflowRun) -> Vec<&str> {
    wr.steps.iter().map(|s| s.step_id.as_str()).collect()
}

/// The path the newest run of the branch step `id` chose (`None` for `otherwise`).
fn chose(wr: &WorkflowRun, id: &str) -> Option<String> {
    let run = wr.steps.iter().rev().find(|s| s.step_id == id).unwrap();
    assert_eq!(run.status, StepRunStatus::Succeeded);
    let path = &run.decision.as_ref().unwrap()["path"];
    path.as_str().map(str::to_string)
}

#[tokio::test]
async fn a_branch_on_project_takes_the_matching_path_then_the_step_after_it() {
    let w = world().await;
    let flow = w
        .workflow(
            "Ship",
            vec![
                branch(
                    "pick",
                    BranchOn::Project,
                    vec![
                        ("Other app", vec![agent("other", "Other", &w.dev, "No.")]),
                        // The project is "Sage": labels match without case.
                        ("sage", vec![agent("dots", "Ship Sage", &w.dev, "Ship it.")]),
                    ],
                    vec![agent("else", "Else", &w.dev, "No.")],
                ),
                agent("after", "After", &w.helper, "Tell the person."),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(ran(&wr), ["pick", "dots", "after"]);
    assert_eq!(chose(&wr, "pick").as_deref(), Some("dots"));
    assert_eq!(wr.current_step, None);
    // Only the path taken ran.
    assert_eq!(w.engine.prompts_of("dev").len(), 1);
}

#[tokio::test]
async fn no_matching_path_takes_otherwise_and_an_empty_otherwise_goes_straight_on() {
    let w = world().await;
    let web = || vec![("Web", vec![agent("web", "Web", &w.dev, "No.")])];
    let flow = w
        .workflow(
            "Otherwise",
            vec![
                branch(
                    "pick",
                    BranchOn::Project,
                    web(),
                    vec![agent("else", "Else", &w.dev, "Do the rest.")],
                ),
                agent("after", "After", &w.helper, "Tell the person."),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(ran(&wr), ["pick", "else", "after"]);
    assert_eq!(chose(&wr, "pick"), None);

    let flow = w
        .workflow(
            "Straight on",
            vec![
                branch("pick", BranchOn::Project, web(), vec![]),
                agent("after", "After", &w.helper, "Tell the person."),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(ran(&wr), ["pick", "after"]);
    assert_eq!(chose(&wr, "pick"), None);

    // A branch that is the last step and takes an empty path ends the run.
    let flow = w
        .workflow(
            "Last",
            vec![branch("pick", BranchOn::Project, web(), vec![])],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(ran(&wr), ["pick"]);
    assert_eq!(wr.current_step, None);
}

/// The flow of the outcome tests: the dev triages and may report `fixed` or `needs_info`.
async fn triage_flow(w: &World) -> String {
    w.workflow(
        "Triage",
        vec![
            reporting("triage", "Triage", &w.dev, &["fixed", "needs_info"]),
            branch(
                "route",
                BranchOn::Outcome,
                vec![
                    ("fixed", vec![agent("ship", "Ship", &w.helper, "Ship.")]),
                    (
                        "needs_info",
                        vec![agent("ask", "Ask", &w.helper, "Ask the person.")],
                    ),
                ],
                vec![agent("else", "Else", &w.helper, "Look again.")],
            ),
        ],
    )
    .await
}

#[tokio::test]
async fn a_branch_on_outcome_takes_the_path_of_the_outcome_reported() {
    let w = world().await;
    w.engine.set(&w.dev.spec.name, vec![Act::Gate]);
    let flow = triage_flow(&w).await;
    let wr = w.start(&flow).await;
    let step_run = w.until(&wr.id, |wr| !wr.steps.is_empty()).await.steps[0].clone();
    // The agent reports while its step runs.
    w.store
        .set_step_outcome(&step_run.id, Some("needs_info"))
        .await
        .unwrap();
    w.engine.gate.notify_one();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(ran(&wr), ["triage", "route", "ask"]);
    assert_eq!(chose(&wr, "route").as_deref(), Some("needs_info"));
    // The hand-off carries the outcome.
    let triage = &wr.steps[0];
    assert_eq!(triage.outcome.as_deref(), Some("needs_info"));
    assert_eq!(
        triage.handoff.as_ref().unwrap().outcome.as_deref(),
        Some("needs_info")
    );
}

#[tokio::test]
async fn a_branch_on_outcome_with_no_outcome_reported_takes_otherwise() {
    let w = world().await;
    let flow = triage_flow(&w).await;
    let wr = w.start(&flow).await;
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(ran(&wr), ["triage", "route", "else"]);
    assert_eq!(chose(&wr, "route"), None);
    assert_eq!(wr.steps[0].handoff.as_ref().unwrap().outcome, None);
}

#[tokio::test]
async fn a_branch_inside_a_path_takes_its_own_path_and_the_run_climbs_out_of_both() {
    let w = world().await;
    let flow = w
        .workflow(
            "Nested",
            vec![
                branch(
                    "outer",
                    BranchOn::Project,
                    vec![(
                        "Sage",
                        vec![branch(
                            "inner",
                            BranchOn::Project,
                            vec![("Other app", vec![agent("other", "Other", &w.dev, "No.")])],
                            vec![agent("deep", "Deep", &w.dev, "Work.")],
                        )],
                    )],
                    vec![],
                ),
                agent("last", "Last", &w.helper, "Tell the person."),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(ran(&wr), ["outer", "inner", "deep", "last"]);
    assert_eq!(chose(&wr, "outer").as_deref(), Some("Sage"));
    assert_eq!(chose(&wr, "inner"), None);
    // The tree in run order: outer, inner, other, deep, last.
    let deep = &w.engine.prompts_of("dev")[0];
    assert!(deep.contains("- Step: Deep (4 of 5)"), "{deep}");
}

#[tokio::test]
async fn the_step_after_a_branch_gets_the_hand_offs_of_the_path_taken() {
    let w = world().await;
    w.engine.set(
        &w.helper.spec.name,
        vec![Act::Finish("Task: ship the fix."), Act::Finish("Reviewed.")],
    );
    w.engine.set(
        &w.dev.spec.name,
        vec![Act::Write("fix.txt", "Fixed on Sage.")],
    );
    let flow = w
        .workflow(
            "Review",
            vec![
                agent("task", "Write the task", &w.helper, "Write it."),
                branch(
                    "pick",
                    BranchOn::Project,
                    vec![
                        ("Other app", vec![agent("skip", "Skipped", &w.dev, "No.")]),
                        ("Sage", vec![agent("fix", "Fix Sage", &w.dev, "Fix it.")]),
                    ],
                    vec![],
                ),
                agent("review", "Review", &w.helper, "Review it."),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(ran(&wr), ["task", "pick", "fix", "review"]);
    // The step on the path gets the hand-off before the branch, numbered in the whole tree.
    let fix = &w.engine.prompts_of("dev")[0];
    for part in [
        "- Step: Fix Sage (4 of 5)",
        "#### Write the task\n\nTask: ship the fix.",
    ] {
        assert!(fix.contains(part), "missing {part:?} in:\n{fix}");
    }
    // The step after the branch gets both, the path's last, and never the path not taken.
    let review = &w.engine.prompts_of("helper")[1];
    for part in [
        "- Step: Review (5 of 5)",
        "### Earlier steps\n\n#### Write the task\n\nTask: ship the fix.\n\n#### Fix Sage\n\nFixed on Sage.",
    ] {
        assert!(review.contains(part), "missing {part:?} in:\n{review}");
    }
    assert!(!review.contains("Skipped"), "{review}");
    // And starts from the path's branch.
    let fixed = &wr.steps[2].handoff.as_ref().unwrap().branches[0].branch;
    let tip = git_out(&w.repo, &["rev-parse", &format!("refs/heads/{fixed}")]);
    let runs = w.store.runs_of_step_run(&wr.steps[3].id).await.unwrap();
    assert_eq!(runs[0].base_commit.as_deref(), Some(tip.as_str()));
}

#[tokio::test]
async fn a_pass_finishes_a_branch_cut_off_before_it_chose() {
    let w = world().await;
    let flow = w
        .workflow(
            "Cut",
            vec![
                check("check"),
                branch(
                    "pick",
                    BranchOn::Project,
                    vec![("Sage", vec![agent("dots", "Sage", &w.dev, "Work.")])],
                    vec![],
                ),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, waits_on(WaitingOn::Check)).await;
    // The check was approved and the branch's step run created, but the move stopped there.
    w.store
        .set_step_run(
            &wr.steps[0].id,
            &dots_core::model::StepRunChange {
                status: Some(StepRunStatus::Succeeded),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    w.store
        .create_step_run(&wr.id, "pick", 1, StepRunStatus::Running)
        .await
        .unwrap();
    w.store
        .set_workflow_run(
            &wr.id,
            &dots_core::model::WorkflowRunChange {
                status: Some(WorkflowRunStatus::Running),
                waiting_on: Some(None),
                current_step: Some(Some("pick".into())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    w.workflows.pass().await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    // The same step run chose: no second attempt.
    assert_eq!(ran(&wr), ["check", "pick", "dots"]);
    assert_eq!(chose(&wr, "pick").as_deref(), Some("Sage"));
}

impl World {
    /// Holds every hand-off job at its gate (`Workflows::hold_jobs`), starts a run of `flow`
    /// and waits until its first step's runs have ended and its hand-off job is held. Gives the
    /// workflow run, the step run's id and the gate.
    async fn held_hand_off(
        &self,
        flow: &str,
    ) -> (WorkflowRun, String, Arc<tokio::sync::Semaphore>) {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        self.workflows.hold_jobs(Some(gate.clone()));
        let wr = self.start(flow).await;
        let step = self.until(&wr.id, |wr| !wr.steps.is_empty()).await.steps[0]
            .id
            .clone();
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while !self.workflows.has_job(&step) {
            assert!(
                std::time::Instant::now() < deadline,
                "no hand-off job started"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        (wr, step, gate)
    }

    /// Waits until the step run `step` has no job in flight.
    async fn job_ended(&self, step: &str) {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while self.workflows.has_job(step) {
            assert!(
                std::time::Instant::now() < deadline,
                "the hand-off job did not end"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }

    /// A two-step flow whose first step writes a file: its hand-off has a commit to make.
    async fn two_steps(&self) -> String {
        self.engine.set(
            &self.dev.spec.name,
            vec![Act::Write("one.txt", "First."), Act::Finish("Second.")],
        );
        self.workflow(
            "Two",
            vec![
                agent("one", "First", &self.dev, "Start."),
                agent("two", "Second", &self.dev, "Go on."),
            ],
        )
        .await
    }
}

#[tokio::test]
async fn a_hand_off_being_built_holds_up_neither_the_loop_nor_a_stop() {
    let w = world().await;
    let flow = w.two_steps().await;
    let (held, step, _gate) = w.held_hand_off(&flow).await;
    let wr = w.store.get_workflow_run(&held.id).await.unwrap();
    assert_eq!(wr.steps[0].status, StepRunStatus::Running);
    assert!(wr.steps[0].handoff.is_none());
    // Another workflow run moves meanwhile: the loop begins its check, and the person's
    // approval ends it.
    let checked = w.workflow("Check", vec![check("check")]).await;
    let other = w.start(&checked).await;
    let other = w.until(&other.id, waits_on(WaitingOn::Check)).await;
    let done = tokio::time::timeout(Duration::from_secs(10), w.approve(&other.id, None))
        .await
        .expect("the approval waited on the job")
        .unwrap();
    assert_eq!(done.status, WorkflowRunStatus::Succeeded);
    // Stopping the run whose hand-off is held does not wait for it.
    let stopped = tokio::time::timeout(Duration::from_secs(10), w.workflows.stop(&held.id))
        .await
        .expect("the stop waited on the job")
        .unwrap();
    assert_eq!(stopped.status, WorkflowRunStatus::Cancelled);
    assert!(w.workflows.has_job(&step));
}

#[tokio::test]
async fn a_stop_during_a_hand_off_wins() {
    let w = world().await;
    let flow = w.two_steps().await;
    let (held, step, gate) = w.held_hand_off(&flow).await;
    w.workflows.stop(&held.id).await.unwrap();
    // The job goes on to build the hand-off, then finds the step cancelled.
    gate.add_permits(1);
    w.job_ended(&step).await;
    w.workflows.pass().await.unwrap();
    let wr = w.store.get_workflow_run(&held.id).await.unwrap();
    assert_eq!(wr.status, WorkflowRunStatus::Cancelled);
    assert_eq!(ran(&wr), ["one"]);
    assert_eq!(wr.steps[0].status, StepRunStatus::Cancelled);
    assert!(wr.steps[0].handoff.is_none());
    assert_eq!(w.engine.prompts_of("dev").len(), 1);
}

#[tokio::test]
async fn a_lost_hand_off_is_built_again_by_a_later_pass() {
    let w = world().await;
    let flow = w.two_steps().await;
    let (held, step, gate) = w.held_hand_off(&flow).await;
    // The job is dropped before it writes, as Dots stopping there would drop it; the jobs
    // after it are let through.
    w.workflows.hold_jobs(None);
    gate.close();
    w.job_ended(&step).await;
    w.workflows.pass().await.unwrap();
    let wr = w
        .until(&held.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(ran(&wr), ["one", "two"]);
    let first = &wr.steps[0].handoff.as_ref().unwrap().branches[0].branch;
    assert_eq!(
        git_out(&w.repo, &["show", &format!("{first}:one.txt")]),
        "First."
    );
    // One attempt of each step: the step run lost its job, not its place.
    assert!(wr.steps.iter().all(|s| s.attempt == 1));
}

#[tokio::test]
async fn a_step_s_runs_get_its_workflow_tools_and_its_prompt_names_them() {
    let w = world().await;
    let mut review = reporting("review", "Review it", &w.dev, &["fixed", "needs_info"]);
    if let StepKind::Agent {
        can_send_back_to, ..
    } = &mut review.kind
    {
        *can_send_back_to = vec!["write".into()];
    }
    let flow = w
        .workflow(
            "Review",
            vec![
                agent("write", "Write the task", &w.helper, "Write it."),
                review,
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    // A step that declares neither outcomes nor send-backs offers no tool, and its prompt names
    // none.
    let write = w.engine.step_tools_of("helper").remove(0).unwrap();
    assert_eq!(write.step_run_id, wr.steps[0].id);
    assert!(Tool::ALL.iter().all(|t| !write.offers(*t)));
    assert!(!w.engine.prompts_of("helper")[0].contains("### Workflow tools"));
    // The review step's run gets its step run, its outcomes and its targets by name.
    assert_eq!(
        w.engine.step_tools_of("dev"),
        [Some(StepTools {
            step_run_id: wr.steps[1].id.clone(),
            step_id: "review".into(),
            outcomes: vec!["fixed".into(), "needs_info".into()],
            send_back_to: vec![SendBackTarget {
                step_id: "write".into(),
                name: "Write the task".into(),
            }],
            delegate: None,
        })]
    );
    let prompt = &w.engine.prompts_of("dev")[0];
    assert!(
        prompt.contains(
            "### Workflow tools\n\n- Report how the step came out with `mcp__dots__report_outcome`, one of: \"fixed\", \"needs_info\". The workflow goes on by the outcome you report.\n- If the work must be redone in an earlier step, send it back with `mcp__dots__send_back` to one of: \"Write the task\", with a note that says what to change.\n\nWhen you are done, end with a summary"
        ),
        "{prompt}"
    );
}

/// A delegate step of `lead` that may start `may_start`, `at_once` at a time and `total` in
/// all.
fn delegating(
    id: &str,
    name: &str,
    lead: &Dot,
    may_start: &[&Dot],
    at_once: u32,
    total: u32,
) -> Step {
    Step {
        id: id.into(),
        name: name.into(),
        kind: StepKind::Agent {
            member: lead.id.clone(),
            job: "Split the work.".into(),
            can_send_back_to: vec![],
            outcomes: vec![],
            delegate: Some(Delegate {
                may_start: may_start.iter().map(|d| d.id.clone()).collect(),
                at_once,
                total,
            }),
        },
    }
}

impl World {
    /// The id of the step run of `step` once the workflow run `id` has one.
    async fn step_run_of(&self, id: &str, step: &str) -> String {
        self.until(id, |wr| wr.steps.iter().any(|s| s.step_id == step))
            .await
            .steps
            .iter()
            .find(|s| s.step_id == step)
            .unwrap()
            .id
            .clone()
    }

    /// The first runs of the members the step run `step` started, in the order started, once
    /// there are `n` of them and each is `status` (a running one, once its workspace is made).
    async fn members_are(&self, step: &str, n: usize, status: RunStatus) -> Vec<Run> {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let mut runs = Vec::new();
            for m in self.store.step_members(step).await.unwrap() {
                runs.push(self.store.get_run(&m.run_id).await.unwrap());
            }
            let ready = |r: &Run| {
                r.status == status && (status != RunStatus::Running || r.workspace_path.is_some())
            };
            if runs.len() == n && runs.iter().all(ready) {
                return runs;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the members are not {status:?}: {runs:#?}"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }

    /// The first run of the step run `step`'s own thread, once it is `status`.
    async fn lead_is(&self, step: &str, status: RunStatus) -> Run {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            let runs = self.store.runs_of_step_run(step).await.unwrap();
            if let Some(run) = runs.first().filter(|r| r.status == status) {
                return run.clone();
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the lead is not {status:?}: {runs:#?}"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }
}

/// What a member's thread ended with, as the hand-off has it.
fn handed(
    dot: &Dot,
    task: &str,
    status: RunStatus,
    summary: &str,
    branch: Option<&str>,
) -> MemberHandoff {
    MemberHandoff {
        dot_id: dot.id.clone(),
        task: task.into(),
        status,
        summary: summary.into(),
        branch: branch.map(str::to_string),
    }
}

#[tokio::test]
async fn a_delegate_step_runs_its_members_at_once_and_is_continued_with_their_results() {
    let w = world().await;
    let light = "Make the icon light.\nUse the light set.";
    w.engine.set(
        &w.helper.spec.name,
        vec![
            Act::Write("plan.txt", "The plan."),
            Act::Start(
                vec![("Dev", light), ("dev", "Make the menu dark.")],
                Box::new(Act::Finish("Started two.")),
            ),
            Act::Finish("Both are in."),
        ],
    );
    w.engine.set(
        &w.dev.spec.name,
        vec![Act::Await("part.txt", "Did my part.")],
    );
    let flow = w
        .workflow(
            "Fix",
            vec![
                agent("plan", "Plan it", &w.helper, "Plan."),
                delegating("split", "Split it", &w.helper, &[&w.dev], 3, 6),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let split = w.step_run_of(&wr.id, "split").await;
    // Both members run at once, each in a worktree of its own, while the lead's run has
    // ended and the step waits for them.
    let members = w.members_are(&split, 2, RunStatus::Running).await;
    assert_ne!(members[0].workspace_path, members[1].workspace_path);
    assert_ne!(members[0].branch, members[1].branch);
    w.lead_is(&split, RunStatus::Succeeded).await;
    w.workflows.pass().await.unwrap();
    let held = w.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(held.steps[1].status, StepRunStatus::Running);
    assert_eq!(w.engine.prompts_of("helper").len(), 2);
    assert_eq!(
        w.engine.answers(),
        [
            Ok("Started \"dev\".".to_string()),
            Ok("Started \"dev\".".to_string())
        ]
    );
    w.engine.held.add_permits(2);
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    let b: Vec<String> = members.iter().map(|m| m.branch.clone().unwrap()).collect();
    // The lead is continued once, with both results.
    let helper = w.engine.prompts_of("helper");
    assert_eq!(helper.len(), 3);
    assert_eq!(
        helper[2],
        format!(
            "The members you started have finished:\n\n\
             #### dev: Make the icon light.\n\nStatus: succeeded\nBranch: {}\n\nDid my part.\n\n\
             #### dev: Make the menu dark.\n\nStatus: succeeded\nBranch: {}\n\nDid my part.\n\n\
             You may start 4 more with `mcp__dots__start_member`, or end with your summary of \
             the step.",
            b[0], b[1]
        )
    );
    // The lead's prompt names its tool; the members' name their part, and they have no tools.
    assert!(
        helper[1].contains(
            "### Workflow tools\n\n- Start other members of the team on parts of the work with \
             `mcp__dots__start_member`, one of: \"dev\". At most 3 may run at once, and 6 in all. \
             End your turn once you have started them: you are told when they finish.\n\n\
             When you are done"
        ),
        "{}",
        helper[1]
    );
    let lead_tools = w.engine.step_tools_of("helper")[1].clone().unwrap();
    assert_eq!(
        lead_tools.delegate,
        Some(DelegateTools {
            may_start: vec![StartableMember {
                dot_id: w.dev.id.clone(),
                name: "dev".into(),
            }],
            at_once: 3,
            total: 6,
        })
    );
    assert_eq!(w.engine.step_tools_of("dev"), [None, None]);
    for prompt in w.engine.prompts_of("dev") {
        for part in [
            "## Workflow\n\nYou are the dev agent of the team \"Sage management\". helper started \
             you on part of one step of the workflow \"Fix\".",
            "- Step: Split it (2 of 2)",
            "### Task\n\nThe tray icon is invisible in dark mode.",
            "When you are done, end with a summary of what you did: helper gets it.",
        ] {
            assert!(prompt.contains(part), "missing {part:?} in:\n{prompt}");
        }
        assert!(!prompt.contains("### Workflow tools"), "{prompt}");
    }
    assert!(w
        .engine
        .prompts_of("dev")
        .iter()
        .any(|p| p.contains(&format!("### Your part\n\n{light}\n\n"))));
    // Both started from the step's start, the plan's branch, and kept their work.
    for branch in &b {
        assert_eq!(
            git_out(&w.repo, &["show", &format!("{branch}:plan.txt")]),
            "The plan."
        );
        assert_eq!(
            git_out(&w.repo, &["show", &format!("{branch}:part.txt")]),
            "Did my part."
        );
    }
    // The lead made no change: the hand-off carries the members' branches.
    let handoff = wr.steps[1].handoff.clone().unwrap();
    assert_eq!(handoff.summary, "Both are in.");
    assert_eq!(
        handoff.members,
        [
            handed(
                &w.dev,
                light,
                RunStatus::Succeeded,
                "Did my part.",
                Some(&b[0])
            ),
            handed(
                &w.dev,
                "Make the menu dark.",
                RunStatus::Succeeded,
                "Did my part.",
                Some(&b[1])
            ),
        ]
    );
    let branches: Vec<&str> = handoff.branches.iter().map(|r| r.branch.as_str()).collect();
    assert_eq!(branches, [b[0].as_str(), b[1].as_str()]);
    assert_eq!(handoff.branches[0].run_id, members[0].id);
    // The plan's run, the lead's two and the members' two.
    assert_eq!(wr.spent_runs, 5);
}

#[tokio::test]
async fn a_lead_that_merged_its_members_work_hands_on_its_own_branch_alone() {
    let w = world().await;
    w.engine.set(
        &w.helper.spec.name,
        vec![
            Act::Start(
                vec![("dev", "Part one.")],
                Box::new(Act::Finish("Started.")),
            ),
            Act::MergeFirst("merged.txt", "Merged."),
        ],
    );
    w.engine
        .set(&w.dev.spec.name, vec![Act::Write("part.txt", "Did it.")]);
    let flow = w
        .workflow(
            "Fix",
            vec![delegating("split", "Split it", &w.helper, &[&w.dev], 3, 6)],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    let split = &wr.steps[0];
    let members = w.members_are(&split.id, 1, RunStatus::Succeeded).await;
    let lead = w.lead_is(&split.id, RunStatus::Succeeded).await;
    let handoff = split.handoff.clone().unwrap();
    let own = lead.branch.clone().unwrap();
    let branches: Vec<&str> = handoff.branches.iter().map(|r| r.branch.as_str()).collect();
    assert_eq!(branches, [own.as_str()]);
    assert_eq!(
        git_out(&w.repo, &["show", &format!("{own}:merged.txt")]),
        "Merged."
    );
    assert_eq!(
        git_out(&w.repo, &["show", &format!("{own}:part.txt")]),
        "Did it."
    );
    // The member's branch is still named in its part of the hand-off.
    let member_branch = members[0].branch.clone().unwrap();
    assert_eq!(
        handoff.members,
        [handed(
            &w.dev,
            "Part one.",
            RunStatus::Succeeded,
            "Did it.",
            Some(&member_branch)
        )]
    );
    assert_eq!(handoff.summary, "Merged.");
}

#[tokio::test]
async fn a_check_shows_and_a_deploy_ships_the_member_branches_its_lead_did_not_merge() {
    let w = world().await;
    w.shipping_to_origin("ok");
    w.engine.set(
        &w.helper.spec.name,
        vec![
            Act::Start(
                vec![
                    ("dev", "Part one."),
                    ("dev", "Part two."),
                    ("dev", "Part three."),
                ],
                Box::new(Act::Finish("Started three.")),
            ),
            Act::MergeFirst("merged.txt", "Merged one part."),
        ],
    );
    // Two members change something; the third changes nothing, so its branch goes with its
    // worktree.
    w.engine.set(
        &w.dev.spec.name,
        vec![
            Act::Write("one.txt", "One done."),
            Act::Write("two.txt", "Two done."),
            Act::Finish("Nothing to change."),
        ],
    );
    let flow = w
        .workflow(
            "Split",
            vec![
                delegating("split", "Split it", &w.helper, &[&w.dev], 3, 6),
                check("check"),
                ship(DeployMode::Push),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, waits_on(WaitingOn::Check)).await;
    let split = newest(&wr, "split").clone();
    let members = w.members_are(&split.id, 3, RunStatus::Succeeded).await;
    let lead = w.lead_is(&split.id, RunStatus::Succeeded).await;
    let own = lead.branch.clone().unwrap();
    let kept: Vec<String> = members
        .iter()
        .filter_map(|m| m.branch.clone())
        .filter(|b| branch_in(&w.repo, b))
        .collect();
    assert_eq!(kept.len(), 2, "{members:#?}");
    let (merged, other) = (&kept[0], &kept[1]);
    // The lead merged the first part: its branch has commits and the first member's tip is
    // in it, but not the second's.
    git_out(&w.repo, &["merge-base", "--is-ancestor", merged, &own]);
    assert_ne!(
        git_out(&w.repo, &["rev-parse", merged]),
        git_out(&w.repo, &["rev-parse", &own])
    );
    // The check shows the lead's branch, then the member branch it did not merge; the
    // merged one and the one that is gone are left out.
    let handoff = split.handoff.clone().unwrap();
    let shown: Vec<&str> = handoff.branches.iter().map(|b| b.branch.as_str()).collect();
    assert_eq!(shown, [own.as_str(), other.as_str()]);
    // The lead's branch is named by the newest run of its thread: the one that merged.
    let named = w.store.get_run(&handoff.branches[0].run_id).await.unwrap();
    assert_eq!(named.root_run_id, lead.id);
    assert_eq!(handoff.members.len(), 3);
    assert_eq!(
        handoff
            .members
            .iter()
            .filter(|m| m.branch.is_none())
            .count(),
        1
    );
    // The approval records the tips of exactly those branches, and the deploy pushes them.
    let tip = |b: &str| git_out(&w.repo, &["rev-parse", &format!("refs/heads/{b}")]);
    let (own_tip, other_tip) = (tip(&own), tip(other));
    let approved = w.approve(&wr.id, None).await.unwrap();
    assert_eq!(
        newest(&approved, "check").decision.as_ref().unwrap()["branches"],
        json!([
            {"branch": own, "commit": own_tip},
            {"branch": other, "commit": other_tip},
        ])
    );
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    let result = newest(&wr, "ship").deploy_result.clone().unwrap();
    assert_eq!(
        result.branches,
        [pushed(&own, &own_tip), pushed(other, &other_tip)]
    );
    assert_eq!(w.remote_tip(&own), Some(own_tip));
    assert_eq!(w.remote_tip(other), Some(other_tip));
    assert_eq!(w.remote_tip(merged), None);
}

#[tokio::test]
async fn start_member_refuses_other_dots_bad_tasks_and_more_than_its_limits_allow() {
    let w = world().await;
    w.engine.set(
        &w.helper.spec.name,
        vec![
            Act::Start(
                vec![
                    ("Helper", "Do it."),
                    ("dev", "  "),
                    ("DEV", "Part one."),
                    ("dev", "Part two."),
                ],
                Box::new(Act::Finish("Started one.")),
            ),
            Act::Start(
                vec![("dev", "Part two.")],
                Box::new(Act::Finish("Started two.")),
            ),
            Act::Start(
                vec![("dev", "Part three.")],
                Box::new(Act::Finish("All in.")),
            ),
        ],
    );
    w.engine
        .set(&w.dev.spec.name, vec![Act::Await("part.txt", "Did it.")]);
    let flow = w
        .workflow(
            "Fix",
            vec![delegating("split", "Split it", &w.helper, &[&w.dev], 1, 2)],
        )
        .await;
    let wr = w.start(&flow).await;
    let split = w.step_run_of(&wr.id, "split").await;
    // The first member holds until the lead's run has made its calls.
    w.lead_is(&split, RunStatus::Succeeded).await;
    w.engine.held.add_permits(2);
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    let started = Ok("Started \"dev\".".to_string());
    assert_eq!(
        w.engine.answers(),
        [
            Err("\"Helper\" is not a member you may start; you may start: \"dev\".".to_string()),
            Err(
                "Give a task of 1 to 8,000 characters that says what the member should do."
                    .to_string()
            ),
            started.clone(),
            Err(
                "Not started: 1 members you started are still running, and at most 1 may run \
                 at once. End your turn; you will be told when they finish."
                    .to_string()
            ),
            started,
            Err("Not started: you have started 2 members, the most this step allows.".to_string()),
        ]
    );
    let helper = w.engine.prompts_of("helper");
    assert_eq!(helper.len(), 3);
    assert!(
        helper[1].contains("#### dev: Part one.\n\nStatus: succeeded\n")
            && helper[1].ends_with(
                "You may start 1 more with `mcp__dots__start_member`, or end with your summary \
                 of the step."
            ),
        "{}",
        helper[1]
    );
    assert!(
        helper[2].contains("#### dev: Part two.\n\nStatus: succeeded\n")
            && !helper[2].contains("Part one.")
            && helper[2]
                .ends_with("You may start no more members: end with your summary of the step."),
        "{}",
        helper[2]
    );
    let handoff = wr.steps[0].handoff.clone().unwrap();
    let tasks: Vec<&str> = handoff.members.iter().map(|m| m.task.as_str()).collect();
    assert_eq!(tasks, ["Part one.", "Part two."]);
    assert_eq!(handoff.summary, "All in.");
}

#[tokio::test]
async fn a_member_past_the_budget_is_refused_and_the_lead_waits_on_budget_to_go_on() {
    let budget = Budget {
        max_runs: 2,
        ..Budget::default()
    };
    let w = world_with(Arc::new(SystemClock), budget).await;
    w.engine.set(
        &w.helper.spec.name,
        vec![
            Act::Start(
                vec![("dev", "Part one."), ("dev", "Part two.")],
                Box::new(Act::Finish("Started.")),
            ),
            Act::Finish("Done."),
        ],
    );
    w.engine
        .set(&w.dev.spec.name, vec![Act::Fail("no disk left")]);
    let flow = w
        .workflow(
            "Fix",
            vec![delegating("split", "Split it", &w.helper, &[&w.dev], 3, 6)],
        )
        .await;
    let wr = w.start(&flow).await;
    // The lead's run and the first member's spent the two runs.
    let held = w.until(&wr.id, waits_on(WaitingOn::Budget)).await;
    assert_eq!(held.error.as_deref(), Some("Budget reached: runs, 2 of 2"));
    assert_eq!(held.steps[0].status, StepRunStatus::Running);
    assert_eq!(
        w.engine.answers(),
        [
            Ok("Started \"dev\".".to_string()),
            Err("Not started: Budget reached: runs, 2 of 2.".to_string()),
        ]
    );
    assert_eq!(w.engine.prompts_of("helper").len(), 1);
    // More budget lets the lead go on, told of the member that failed: not fatal to the step.
    w.workflows.add_budget(&wr.id).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    let told = &w.engine.prompts_of("helper")[1];
    assert!(
        told.contains("#### dev: Part one.\n\nStatus: failed\nBranch: none\n\nno disk left\n\n"),
        "{told}"
    );
    let handoff = wr.steps[0].handoff.clone().unwrap();
    assert_eq!(
        handoff.members,
        [handed(
            &w.dev,
            "Part one.",
            RunStatus::Failed,
            "no disk left",
            None
        )]
    );
    assert!(handoff.branches.is_empty());
    assert_eq!(wr.spent_runs, 3);
}

#[tokio::test]
async fn a_lead_that_fails_cancels_its_running_members_and_fails_the_step() {
    let w = world().await;
    w.engine.set(
        &w.helper.spec.name,
        vec![Act::Start(
            vec![("dev", "Part one.")],
            Box::new(Act::Fail("lost the plan")),
        )],
    );
    w.engine.set(&w.dev.spec.name, vec![Act::Hold]);
    let flow = w
        .workflow(
            "Fix",
            vec![delegating("split", "Split it", &w.helper, &[&w.dev], 3, 6)],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, waits_on(WaitingOn::StepFailed)).await;
    assert_eq!(wr.error.as_deref(), Some("Split it: lost the plan"));
    assert_eq!(wr.steps[0].status, StepRunStatus::Failed);
    let member = w.store.step_members(&wr.steps[0].id).await.unwrap()[0]
        .run_id
        .clone();
    common::wait_status(&w.store, &member, RunStatus::Cancelled, 60).await;
    assert_eq!(w.engine.prompts_of("helper").len(), 1);
}

#[tokio::test]
async fn stopping_a_delegate_step_cancels_its_members() {
    let w = world().await;
    w.engine.set(
        &w.helper.spec.name,
        vec![Act::Start(
            vec![("dev", "Part one.")],
            Box::new(Act::Finish("Started.")),
        )],
    );
    w.engine.set(&w.dev.spec.name, vec![Act::Hold]);
    let flow = w
        .workflow(
            "Fix",
            vec![delegating("split", "Split it", &w.helper, &[&w.dev], 3, 6)],
        )
        .await;
    let wr = w.start(&flow).await;
    let split = w.step_run_of(&wr.id, "split").await;
    let member = w.members_are(&split, 1, RunStatus::Running).await.remove(0);
    let stopped = w.workflows.stop(&wr.id).await.unwrap();
    assert_eq!(stopped.status, WorkflowRunStatus::Cancelled);
    assert_eq!(stopped.steps[0].status, StepRunStatus::Cancelled);
    common::wait_status(&w.store, &member.id, RunStatus::Cancelled, 60).await;
    assert_eq!(w.engine.prompts_of("helper").len(), 1);
}

#[tokio::test]
async fn a_member_interrupted_by_a_restart_is_reported_as_failed_and_the_lead_goes_on() {
    let w = world().await;
    w.engine.set(
        &w.helper.spec.name,
        vec![
            Act::Start(
                vec![("dev", "Part one.")],
                Box::new(Act::Finish("Started.")),
            ),
            Act::Finish("Went on without it."),
        ],
    );
    w.engine.set(&w.dev.spec.name, vec![Act::Hold]);
    let flow = w
        .workflow(
            "Fix",
            vec![delegating("split", "Split it", &w.helper, &[&w.dev], 3, 6)],
        )
        .await;
    let wr = w.start(&flow).await;
    let split = w.step_run_of(&wr.id, "split").await;
    let member = w.members_are(&split, 1, RunStatus::Running).await.remove(0);
    w.lead_is(&split, RunStatus::Succeeded).await;
    // As Dots does at start after a crash: the running member is failed as interrupted, and
    // a new loop's first pass continues the lead, which waited for it.
    let interrupted = w.store.recover_interrupted().await.unwrap();
    assert_eq!(interrupted, std::slice::from_ref(&member.id));
    let fresh = Workflows::new(
        w.store.clone(),
        new_bus(),
        w.runner.clone(),
        Arc::new(SystemClock),
    );
    fresh.pass().await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    let helper = w.engine.prompts_of("helper");
    assert_eq!(helper.len(), 2, "the lead was continued once");
    assert!(
        helper[1].contains("#### dev: Part one.\n\nStatus: failed\nBranch: ")
            && helper[1].contains("\n\ninterrupted\n\n"),
        "{}",
        helper[1]
    );
    let handoff = wr.steps[0].handoff.clone().unwrap();
    assert_eq!(handoff.summary, "Went on without it.");
    assert_eq!(handoff.members.len(), 1);
    assert_eq!(handoff.members[0].status, RunStatus::Failed);
    assert_eq!(handoff.members[0].summary, "interrupted");
}

/// An agent step of `member` that may send the work back to the steps `to`.
fn sending_back(id: &str, name: &str, member: &Dot, job: &str, to: &[&str]) -> Step {
    let mut step = agent(id, name, member, job);
    if let StepKind::Agent {
        can_send_back_to, ..
    } = &mut step.kind
    {
        *can_send_back_to = to.iter().map(|s| s.to_string()).collect();
    }
    step
}

/// The step runs of the workflow run as (step id, attempt, status), in the order made.
fn attempts(wr: &WorkflowRun) -> Vec<(&str, u32, StepRunStatus)> {
    wr.steps
        .iter()
        .map(|s| (s.step_id.as_str(), s.attempt, s.status))
        .collect()
}

/// The newest step run of `step`.
fn newest<'a>(wr: &'a WorkflowRun, step: &str) -> &'a StepRun {
    wr.steps.iter().rev().find(|s| s.step_id == step).unwrap()
}

/// Waiting on the check `step`, which has had `n` step runs.
fn at(step: &'static str, n: usize) -> impl Fn(&WorkflowRun) -> bool {
    move |wr| {
        waits_on(WaitingOn::Check)(wr)
            && wr.current_step.as_deref() == Some(step)
            && wr.steps.iter().filter(|s| s.step_id == step).count() == n
    }
}

/// Waiting on the check `check`, which has had `n` step runs.
fn at_check(n: usize) -> impl Fn(&WorkflowRun) -> bool {
    at("check", n)
}

impl World {
    /// The runs of the step run `step_run`, oldest first.
    async fn runs_of(&self, step_run: &str) -> Vec<Run> {
        self.store.runs_of_step_run(step_run).await.unwrap()
    }

    /// As `until`, with a pass of `workflows` before each look, for a test whose loop is
    /// stopped.
    async fn until_passing(
        &self,
        workflows: &Workflows,
        id: &str,
        done: impl Fn(&WorkflowRun) -> bool,
    ) -> WorkflowRun {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        loop {
            workflows.pass().await.unwrap();
            let wr = self.store.get_workflow_run(id).await.unwrap();
            if done(&wr) {
                return wr;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "workflow run {id} did not get there: {wr:#?}"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }
}

/// The person's send-back of `note` from the check `check` to the step `to`.
fn by_person(check: &str, to: &str, note: &str, held: bool) -> Option<SendBack> {
    Some(SendBack {
        from: check.into(),
        to: to.into(),
        note: note.into(),
        by: SentBy::Person,
        held,
    })
}

#[tokio::test]
async fn the_person_sends_back_from_a_check_and_the_step_goes_on_in_its_own_session() {
    let w = world().await;
    w.engine.set(
        &w.helper.spec.name,
        vec![Act::Finish("Task: make the icon visible.")],
    );
    w.engine.set(
        &w.dev.spec.name,
        vec![
            Act::Write("icon.txt", "Made the icon visible."),
            Act::Write("dark.txt", "Used the light set."),
        ],
    );
    let flow = w.standard().await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, at_check(1)).await;
    let first = w.runs_of(&newest(&wr, "dev").id).await.remove(0);
    let branch = first.branch.clone().unwrap();

    let sent = w
        .send_back(&wr.id, "  Use the light icon set.  ")
        .await
        .unwrap();
    assert_eq!(sent.send_backs, 1);
    assert_eq!(
        sent.send_back,
        by_person("check", "dev", "Use the light icon set.", false)
    );
    let decided = &sent.steps[2];
    assert_eq!(decided.status, StepRunStatus::Succeeded);
    assert_eq!(
        decided.decision,
        Some(json!({"decision": "sent_back", "note": "Use the light icon set."}))
    );

    // The check waits again, on the dev's new work.
    let wr = w.until(&wr.id, at_check(2)).await;
    assert_eq!(
        attempts(&wr),
        [
            ("helper", 1, StepRunStatus::Succeeded),
            ("dev", 1, StepRunStatus::Succeeded),
            ("check", 1, StepRunStatus::Succeeded),
            ("dev", 2, StepRunStatus::Succeeded),
            ("check", 2, StepRunStatus::Waiting),
        ]
    );
    // The dev's run is continued for the new step run: the same session, worktree and branch.
    let again = newest(&wr, "dev");
    let runs = w.runs_of(&again.id).await;
    assert_eq!(runs.len(), 1);
    let run = &runs[0];
    assert_eq!(run.parent_run_id.as_deref(), Some(first.id.as_str()));
    assert_eq!(run.root_run_id, first.root_run_id);
    assert!(run.session_id.is_some());
    assert_eq!(run.session_id, first.session_id);
    assert_eq!(run.workspace_path, first.workspace_path);
    assert_eq!(run.branch.as_deref(), Some(branch.as_str()));
    assert_eq!(run.workflow_step_run_id.as_deref(), Some(again.id.as_str()));
    assert_eq!(
        w.engine.prompts_of("dev")[1],
        "The person sent this back: Use the light icon set."
    );
    // Its new commit is on that branch, over the first one, and the hand-off names it.
    let handoff = again.handoff.clone().unwrap();
    assert_eq!(handoff.summary, "Used the light set.");
    assert_eq!(handoff.branches.len(), 1);
    assert_eq!(handoff.branches[0].branch, branch);
    assert_eq!(
        git_out(&w.repo, &["show", &format!("{branch}:dark.txt")]),
        "Used the light set."
    );
    assert_eq!(
        git_out(&w.repo, &["show", &format!("{branch}:icon.txt")]),
        "Made the icon visible."
    );
    // The step before the target does not run again.
    assert_eq!(w.engine.prompts_of("helper").len(), 1);
    assert_eq!(wr.spent_runs, 3);

    let done = w.approve(&wr.id, None).await.unwrap();
    assert_eq!(done.status, WorkflowRunStatus::Succeeded);
    assert_eq!(
        done.steps[4].handoff.as_ref().unwrap().summary,
        "Used the light set."
    );
}

#[tokio::test]
async fn an_agents_send_back_goes_when_its_step_ends_and_the_steps_between_run_again() {
    let w = world().await;
    w.engine.set(
        &w.dev.spec.name,
        vec![
            Act::Write("icon.txt", "Made the icon visible."),
            Act::Write("dark.txt", "Used the light set."),
        ],
    );
    w.engine.set(
        &w.helper.spec.name,
        vec![
            Act::Write("notes.txt", "Notes one."),
            // The review makes no change: its branch is gone when it runs again.
            Act::SendBack(
                "build",
                "Use the light set.",
                Box::new(Act::Finish("Sent it back.")),
            ),
            Act::Write("notes.txt", "Notes two."),
            Act::Write("review.txt", "Looks right now."),
        ],
    );
    let flow = w
        .workflow(
            "Fix",
            vec![
                agent("build", "Build", &w.dev, "Fix the bug."),
                agent("notes", "Notes", &w.helper, "Write release notes."),
                sending_back("review", "Review", &w.helper, "Review it.", &["build"]),
                check("check"),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, at_check(1)).await;
    assert_eq!(
        attempts(&wr),
        [
            ("build", 1, StepRunStatus::Succeeded),
            ("notes", 1, StepRunStatus::Succeeded),
            ("review", 1, StepRunStatus::Succeeded),
            ("build", 2, StepRunStatus::Succeeded),
            ("notes", 2, StepRunStatus::Succeeded),
            ("review", 2, StepRunStatus::Succeeded),
            ("check", 1, StepRunStatus::Waiting),
        ]
    );
    assert_eq!(
        w.engine.answers(),
        [Ok(
            "Sent back to \"Build\". End your turn now: the work goes back when you finish."
                .to_string()
        )]
    );
    assert_eq!(wr.send_backs, 1);
    assert_eq!(
        wr.send_back,
        Some(SendBack {
            from: "review".into(),
            to: "build".into(),
            note: "Use the light set.".into(),
            by: SentBy::Agent,
            held: false,
        })
    );
    assert_eq!(
        wr.steps[2].send_back,
        Some(json!({"to": "build", "note": "Use the light set."}))
    );
    assert_eq!(newest(&wr, "review").send_back, None);
    assert_eq!(
        w.engine.prompts_of("dev")[1],
        "The step \"Review\" sent this back: Use the light set."
    );

    // The notes step is continued with the build's new hand-off. Its branch still has its
    // notes, so it is told to merge the new work in.
    let build = newest(&wr, "build").handoff.clone().unwrap().branches[0]
        .branch
        .clone();
    let notes_first = w.runs_of(&wr.steps[1].id).await.remove(0);
    let notes_again = w.runs_of(&newest(&wr, "notes").id).await.remove(0);
    assert_eq!(
        notes_again.parent_run_id.as_deref(),
        Some(notes_first.id.as_str())
    );
    assert_eq!(notes_again.session_id, notes_first.session_id);
    let helper = w.engine.prompts_of("helper");
    assert_eq!(helper.len(), 4);
    assert_eq!(
        helper[2],
        format!(
            "Work was sent back to \"Build\" and has run again. Its new hand-off:\n\n\
             Used the light set.\n\nBranches: {build}\n\nYour branch still has your earlier \
             commits: merge \"{build}\" into it before you go on.\n\nUpdate your work to match, \
             then end with a new summary."
        )
    );

    // The review is continued with the notes' new hand-off. Its branch was gone, so its
    // worktree is made again on that branch from the notes' new tip.
    let notes = newest(&wr, "notes").handoff.clone().unwrap().branches[0]
        .branch
        .clone();
    assert_eq!(
        helper[3],
        format!(
            "Work was sent back to \"Build\" and has run again. Its new hand-off:\n\n\
             Notes two.\n\nBranches: {notes}\n\nUpdate your work to match, then end with a new \
             summary."
        )
    );
    let review_first = w.runs_of(&wr.steps[2].id).await.remove(0);
    let review_again = w.runs_of(&newest(&wr, "review").id).await.remove(0);
    assert_eq!(
        review_again.parent_run_id.as_deref(),
        Some(review_first.id.as_str())
    );
    assert_eq!(review_again.session_id, review_first.session_id);
    assert_eq!(review_again.branch, review_first.branch);
    let tip = git_out(&w.repo, &["rev-parse", &format!("refs/heads/{notes}")]);
    assert_eq!(review_again.base_commit.as_deref(), Some(tip.as_str()));
    assert_ne!(review_again.base_commit, review_first.base_commit);
    let review = review_again.branch.clone().unwrap();
    assert_eq!(
        git_out(&w.repo, &["show", &format!("{review}:notes.txt")]),
        "Notes two."
    );
    assert_eq!(
        git_out(&w.repo, &["show", &format!("{review}:review.txt")]),
        "Looks right now."
    );
    // The check shows the review's new work.
    assert_eq!(
        newest(&wr, "review").handoff.as_ref().unwrap().branches[0].branch,
        review
    );
}

#[tokio::test]
async fn past_the_cap_a_send_back_waits_until_the_person_allows_one_more() {
    let budget = Budget {
        max_runs: 50,
        ..Budget::default()
    };
    let w = world_with(Arc::new(SystemClock), budget).await;
    w.engine.set(
        &w.dev.spec.name,
        vec![Act::Write("icon.txt", "Made the icon visible.")],
    );
    let flow = w
        .workflow(
            "Fix",
            vec![
                agent("dev", "Fix it", &w.dev, "Fix the bug."),
                check("check"),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    for (n, note) in ["One.", "Two.", "Three."].into_iter().enumerate() {
        w.until(&wr.id, at_check(n + 1)).await;
        let sent = w.send_back(&wr.id, note).await.unwrap();
        assert_eq!(sent.send_backs, n as u32 + 1);
    }
    w.until(&wr.id, at_check(4)).await;

    // The fourth is held: nothing goes back, and a pass leaves it waiting.
    let held = w.send_back(&wr.id, "Four.").await.unwrap();
    assert_eq!(held.status, WorkflowRunStatus::Waiting);
    assert_eq!(held.waiting_on, Some(WaitingOn::SendBacks));
    assert_eq!(
        held.error.as_deref(),
        Some("Send-back limit reached: 3 of 3")
    );
    assert_eq!(held.send_backs, 3);
    assert_eq!(held.send_back, by_person("check", "dev", "Four.", true));
    assert_eq!(held.current_step.as_deref(), Some("check"));
    w.workflows.pass().await.unwrap();
    let still = w.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(still.waiting_on, Some(WaitingOn::SendBacks));
    assert_eq!(still.steps.iter().filter(|s| s.step_id == "dev").count(), 4);
    assert_eq!(w.engine.prompts_of("dev").len(), 4);
    assert!(matches!(
        w.workflows.approve_check(&wr.id, None, "", &[]).await,
        Err(Error::Conflict(_))
    ));

    let allowed = w.workflows.allow_send_back(&wr.id).await.unwrap();
    assert_eq!(allowed.budget.max_send_backs, 4);
    assert_eq!(allowed.send_backs, 4);
    assert_eq!(allowed.status, WorkflowRunStatus::Running);
    assert_eq!(allowed.error, None);
    assert_eq!(allowed.send_back, by_person("check", "dev", "Four.", false));
    let wr = w.until(&wr.id, at_check(5)).await;
    let dev = w.engine.prompts_of("dev");
    assert_eq!(dev.len(), 5);
    assert_eq!(dev[4], "The person sent this back: Four.");
    match w.workflows.allow_send_back(&wr.id).await {
        Err(Error::Conflict(m)) => assert_eq!(
            m,
            format!(
                "workflow run {} is not waiting at its send-back limit",
                wr.id
            )
        ),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_send_back_past_the_runs_limit_waits_on_budget_then_goes_on() {
    let budget = Budget {
        max_runs: 1,
        ..Budget::default()
    };
    let w = world_with(Arc::new(SystemClock), budget).await;
    w.engine.set(
        &w.dev.spec.name,
        vec![
            Act::Write("icon.txt", "Made the icon visible."),
            Act::Write("dark.txt", "Used the light set."),
        ],
    );
    let flow = w
        .workflow(
            "Fix",
            vec![
                agent("dev", "Fix it", &w.dev, "Fix the bug."),
                check("check"),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    w.until(&wr.id, at_check(1)).await;
    let sent = w.send_back(&wr.id, "Use the light set.").await.unwrap();
    assert_eq!(sent.waiting_on, Some(WaitingOn::Budget));
    assert_eq!(sent.error.as_deref(), Some("Budget reached: runs, 1 of 1"));
    assert_eq!(sent.send_backs, 1);
    assert_eq!(sent.current_step.as_deref(), Some("dev"));
    let again = newest(&sent, "dev");
    assert_eq!((again.attempt, again.status), (2, StepRunStatus::Running));
    assert!(w.runs_of(&again.id).await.is_empty());

    w.workflows.add_budget(&wr.id).await.unwrap();
    w.until(&wr.id, at_check(2)).await;
    let dev = w.engine.prompts_of("dev");
    assert_eq!(dev.len(), 2);
    assert_eq!(dev[1], "The person sent this back: Use the light set.");
}

#[tokio::test]
async fn a_step_whose_session_cannot_be_resumed_starts_again_and_says_so() {
    let w = world().await;
    w.engine.set(
        &w.helper.spec.name,
        vec![
            Act::NoSession(None, "Task: make the icon visible."),
            Act::Finish("Task: use the light set."),
        ],
    );
    w.engine.set(
        &w.dev.spec.name,
        vec![
            Act::NoSession(Some("icon.txt"), "Made the icon visible."),
            Act::Write("dark.txt", "Used the light set."),
        ],
    );
    let flow = w
        .workflow(
            "Fix",
            vec![
                agent("helper", "Write the task", &w.helper, "Write the task."),
                Step {
                    id: "task".into(),
                    name: "Task check".into(),
                    kind: StepKind::Check { send_back_to: None },
                },
                agent("dev", "Fix it", &w.dev, "Fix the bug."),
                check("check"),
            ],
        )
        .await;
    let wr = w.start(&flow).await;

    // The helper made no change: it starts again as it started the first time.
    let wr = w.until(&wr.id, at("task", 1)).await;
    w.send_back(&wr.id, "Say which icon.").await.unwrap();
    let wr = w.until(&wr.id, at("task", 2)).await;
    let helper = newest(&wr, "helper");
    assert_eq!(
        helper.notice.as_deref(),
        Some("Started again from the start: the earlier session could not be resumed.")
    );
    let fresh = w.runs_of(&helper.id).await.remove(0);
    assert_eq!(fresh.trigger, TriggerKind::Workflow);
    assert_eq!(fresh.parent_run_id, None);
    assert_eq!(
        fresh.payload.as_ref().unwrap()["workflow"]["start"],
        json!(null)
    );
    let prompt = &w.engine.prompts_of("helper")[1];
    for part in [
        "## Workflow",
        "### Your job in this step\n\nWrite the task.",
        "### Sent back\n\nThe person sent this back: Say which icon.\n\nWhen you are done",
    ] {
        assert!(prompt.contains(part), "missing {part:?} in:\n{prompt}");
    }
    assert_eq!(wr.steps[0].notice, None);

    // The dev made a change: it starts again from its old branch.
    w.approve(&wr.id, None).await.unwrap();
    let wr = w.until(&wr.id, at_check(1)).await;
    let old = w.runs_of(&newest(&wr, "dev").id).await.remove(0);
    let old_branch = old.branch.clone().unwrap();
    w.send_back(&wr.id, "Use the light set.").await.unwrap();
    let wr = w.until(&wr.id, at_check(2)).await;
    let dev = newest(&wr, "dev");
    assert_eq!(
        dev.notice.clone(),
        Some(format!(
            "Started again from \"{old_branch}\": the earlier session could not be resumed."
        ))
    );
    let fresh = w.runs_of(&dev.id).await.remove(0);
    assert_eq!(fresh.parent_run_id, None);
    assert_eq!(
        fresh.payload.as_ref().unwrap()["workflow"]["start"],
        json!(format!("refs/heads/{old_branch}"))
    );
    let prompt = &w.engine.prompts_of("dev")[1];
    assert!(
        prompt.contains("### Sent back\n\nThe person sent this back: Use the light set.\n\n"),
        "{prompt}"
    );
    let branch = &dev.handoff.as_ref().unwrap().branches[0].branch;
    assert_ne!(branch, &old_branch);
    assert_eq!(
        git_out(&w.repo, &["show", &format!("{branch}:icon.txt")]),
        "Made the icon visible."
    );
    assert_eq!(
        git_out(&w.repo, &["show", &format!("{branch}:dark.txt")]),
        "Used the light set."
    );
}

#[tokio::test]
async fn a_send_back_to_a_delegate_step_continues_the_delegating_run() {
    let w = world().await;
    w.engine.set(
        &w.helper.spec.name,
        vec![
            Act::Start(
                vec![("dev", "Part one.")],
                Box::new(Act::Finish("Started.")),
            ),
            Act::Finish("Merged it."),
            Act::Finish("Took the note."),
        ],
    );
    w.engine.set(
        &w.dev.spec.name,
        vec![Act::Write("one.txt", "Part one done.")],
    );
    let flow = w
        .workflow(
            "Fix",
            vec![
                delegating("split", "Split it", &w.helper, &[&w.dev], 3, 6),
                check("check"),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, at_check(1)).await;
    let first = newest(&wr, "split");
    let members = w.store.step_members(&first.id).await.unwrap();
    assert_eq!(members.len(), 1);
    let lead = w
        .runs_of(&first.id)
        .await
        .into_iter()
        .rev()
        .find(|r| !members.iter().any(|m| m.run_id == r.root_run_id))
        .unwrap();
    w.send_back(&wr.id, "Split it finer.").await.unwrap();
    let wr = w.until(&wr.id, at_check(2)).await;
    let again = newest(&wr, "split");
    let runs = w.runs_of(&again.id).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].parent_run_id.as_deref(), Some(lead.id.as_str()));
    assert_eq!(runs[0].root_run_id, lead.root_run_id);
    assert_eq!(runs[0].session_id, lead.session_id);
    assert_eq!(
        w.engine.prompts_of("helper")[2],
        "The person sent this back: Split it finer."
    );
    // It started no member this time; the member did not run again.
    assert!(w.store.step_members(&again.id).await.unwrap().is_empty());
    assert_eq!(w.engine.prompts_of("dev").len(), 1);
    let handoff = again.handoff.clone().unwrap();
    assert_eq!(handoff.summary, "Took the note.");
    assert!(handoff.members.is_empty());
}

#[tokio::test]
async fn send_back_check_refuses_a_bad_note_a_check_with_no_agent_before_it_and_no_check() {
    let w = world().await;
    let flow = w.workflow("Look", vec![check("check")]).await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, at_check(1)).await;
    let long = "x".repeat(8_001);
    for note in ["", "   ", long.as_str()] {
        match w.send_back(&wr.id, note).await {
            Err(Error::Invalid(m)) => assert_eq!(m, "the note must be 1 to 8,000 characters"),
            other => panic!("{other:?}"),
        }
    }
    match w.send_back(&wr.id, "Redo it.").await {
        Err(Error::Invalid(m)) => assert_eq!(
            m,
            "the check \"Your check\" has no agent step before it to send the work back to"
        ),
        other => panic!("{other:?}"),
    }
    // Nothing was written.
    let still = w.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(attempts(&still), [("check", 1, StepRunStatus::Waiting)]);
    assert_eq!(still.waiting_on, Some(WaitingOn::Check));
    assert_eq!((still.send_backs, still.send_back.clone()), (0, None));

    w.approve(&wr.id, None).await.unwrap();
    match w.send_back(&wr.id, "Redo it.").await {
        Err(Error::Conflict(m)) => assert_eq!(
            m,
            format!("workflow run {} is not waiting for a check", wr.id)
        ),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_send_back_cut_off_by_a_restart_is_finished_by_the_next_pass_once() {
    let w = world().await;
    w.engine.set(
        &w.dev.spec.name,
        vec![
            Act::Write("icon.txt", "Made the icon visible."),
            Act::Write("light.txt", "Used the light set."),
            Act::Write("dark.txt", "Used the dark set."),
        ],
    );
    let flow = w
        .workflow(
            "Fix",
            vec![
                agent("dev", "Fix it", &w.dev, "Fix the bug."),
                check("check"),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, at_check(1)).await;
    // Dots stops, with the loop, while the person's send-back is half made.
    w.stop_loop.cancel();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let restarted = || {
        Workflows::new(
            w.store.clone(),
            new_bus(),
            w.runner.clone(),
            Arc::new(SystemClock),
        )
    };
    let decide = |note: &str| StepRunChange {
        status: Some(StepRunStatus::Succeeded),
        decision: Some(Some(json!({"decision": "sent_back", "note": note}))),
        ..Default::default()
    };

    // Cut off after the check was decided: nothing went back yet. The next passes send it
    // back once.
    w.store
        .set_step_run(&newest(&wr, "check").id, &decide("Use the light set."))
        .await
        .unwrap();
    w.store
        .set_workflow_run(
            &wr.id,
            &WorkflowRunChange {
                status: Some(WorkflowRunStatus::Running),
                waiting_on: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let first = restarted();
    first.pass().await.unwrap();
    first.pass().await.unwrap();
    let moved = w.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(moved.send_backs, 1);
    assert_eq!(moved.current_step.as_deref(), Some("dev"));
    let dev2 = newest(&moved, "dev").clone();
    assert_eq!(dev2.attempt, 2);
    assert_eq!(w.runs_of(&dev2.id).await.len(), 1);
    let wr = w.until_passing(&first, &wr.id, at_check(2)).await;
    assert_eq!(
        w.engine.prompts_of("dev")[1],
        "The person sent this back: Use the light set."
    );

    // Cut off after the dev's new step run was made and the run moved to it, before its run
    // was queued: the next passes queue the continuation once.
    w.store
        .set_step_run(&newest(&wr, "check").id, &decide("Use the dark set."))
        .await
        .unwrap();
    let dev3 = w
        .store
        .create_step_run(&wr.id, "dev", 3, StepRunStatus::Running)
        .await
        .unwrap();
    w.store
        .set_workflow_run(
            &wr.id,
            &WorkflowRunChange {
                status: Some(WorkflowRunStatus::Running),
                waiting_on: Some(None),
                current_step: Some(Some("dev".into())),
                send_backs: Some(2),
                send_back: Some(by_person("check", "dev", "Use the dark set.", false)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let second = restarted();
    second.pass().await.unwrap();
    second.pass().await.unwrap();
    let runs = w.runs_of(&dev3.id).await;
    assert_eq!(runs.len(), 1);
    let dev2_run = w.runs_of(&dev2.id).await.remove(0);
    assert_eq!(runs[0].parent_run_id.as_deref(), Some(dev2_run.id.as_str()));
    let wr = w.until_passing(&second, &wr.id, at_check(3)).await;
    assert_eq!(wr.send_backs, 2);
    assert_eq!(
        w.engine.prompts_of("dev")[2],
        "The person sent this back: Use the dark set."
    );
    assert_eq!(
        attempts(&wr),
        [
            ("dev", 1, StepRunStatus::Succeeded),
            ("check", 1, StepRunStatus::Succeeded),
            ("dev", 2, StepRunStatus::Succeeded),
            ("check", 2, StepRunStatus::Succeeded),
            ("dev", 3, StepRunStatus::Succeeded),
            ("check", 3, StepRunStatus::Waiting),
        ]
    );
    assert_eq!(w.engine.prompts_of("dev").len(), 3);
}

#[tokio::test]
async fn a_delegate_step_sent_back_counts_its_total_over_every_attempt() {
    let w = world().await;
    w.engine.set(
        &w.helper.spec.name,
        vec![
            Act::Start(
                vec![("dev", "Part one.")],
                Box::new(Act::Finish("Started.")),
            ),
            Act::Finish("Merged it."),
            Act::Start(
                vec![("dev", "Part two."), ("dev", "Part three.")],
                Box::new(Act::Finish("Started again.")),
            ),
            Act::Finish("Done again."),
        ],
    );
    w.engine.set(
        &w.dev.spec.name,
        vec![Act::Finish("Part one done."), Act::Finish("Part two done.")],
    );
    let flow = w
        .workflow(
            "Fix",
            vec![
                delegating("split", "Split it", &w.helper, &[&w.dev], 2, 2),
                check("check"),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    w.until(&wr.id, at_check(1)).await;
    let helper = w.engine.prompts_of("helper");
    assert!(
        helper[1].ends_with(
            "You may start 1 more with `mcp__dots__start_member`, or end with your summary of \
             the step."
        ),
        "{}",
        helper[1]
    );
    w.send_back(&wr.id, "Split it finer.").await.unwrap();
    let wr = w.until(&wr.id, at_check(2)).await;
    // The member started in the first attempt counts: the second may start one more, and the
    // third is refused.
    assert_eq!(
        w.engine.answers(),
        [
            Ok("Started \"dev\".".to_string()),
            Ok("Started \"dev\".".to_string()),
            Err("Not started: you have started 2 members, the most this step allows.".into()),
        ]
    );
    let again = newest(&wr, "split");
    assert_eq!(w.store.step_members(&again.id).await.unwrap().len(), 1);
    assert_eq!(w.store.step_members_started(&again.id).await.unwrap(), 2);
    let helper = w.engine.prompts_of("helper");
    assert_eq!(helper.len(), 4);
    assert_eq!(helper[2], "The person sent this back: Split it finer.");
    assert!(
        helper[3].contains("#### dev: Part two.")
            && helper[3]
                .ends_with("You may start no more members: end with your summary of the step."),
        "{}",
        helper[3]
    );
    assert_eq!(again.handoff.as_ref().unwrap().summary, "Done again.");
}

/// A deploy step `ship` named "Ship".
fn ship(mode: DeployMode) -> Step {
    Step {
        id: "ship".into(),
        name: "Ship".into(),
        kind: StepKind::Deploy { mode },
    }
}

impl World {
    /// Gives the project a bare repository as its `origin` (with `main` pushed) and the
    /// workflows the fake `gh` answering as `gh_mode`, which logs its calls to `gh.log`. Its
    /// settings go to its own calls only, so tests running at once do not share them.
    fn shipping_to_origin(&self, gh_mode: &str) {
        git_out(self.dir.path(), &["init", "-q", "--bare", "remote.git"]);
        let remote = self.dir.path().join("remote.git");
        git_out(
            &self.repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git_out(&self.repo, &["push", "-q", "origin", "main"]);
        let log = self.dir.path().join("gh.log");
        self.workflows.set_deployer(Deployer {
            gh: PathBuf::from(env!("CARGO_BIN_EXE_fake-gh")),
            env: vec![
                ("FAKE_GH_LOG".into(), log.to_string_lossy().to_string()),
                ("FAKE_GH_MODE".into(), gh_mode.into()),
            ],
        });
    }

    /// The commit `branch` points at on `origin`, if it was pushed there.
    fn remote_tip(&self, branch: &str) -> Option<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(self.dir.path().join("remote.git"))
            .args(["rev-parse", "--verify", "--quiet"])
            .arg(format!("refs/heads/{branch}"))
            .output()
            .unwrap();
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// The fake `gh`'s calls, one argument list each.
    fn gh_calls(&self) -> Vec<Vec<String>> {
        let text = std::fs::read_to_string(self.dir.path().join("gh.log")).unwrap_or_default();
        text.lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// The flow "Ship": the dev fixes the bug (writing `fix.txt`), the person checks, then the
    /// step "Ship" deploys as `mode`.
    async fn ship_flow(&self, mode: DeployMode) -> String {
        self.engine.set(
            &self.dev.spec.name,
            vec![Act::Write("fix.txt", "Fixed the icon.")],
        );
        self.workflow(
            "Ship",
            vec![
                agent("dev", "Fix it", &self.dev, "Fix the bug."),
                check("check"),
                ship(mode),
            ],
        )
        .await
    }

    /// Starts a run of `flow` (a test run with `test`) and waits for its check. Gives the
    /// workflow run, the dev's branch and its tip.
    async fn at_the_check(&self, flow: &str, test: bool) -> (WorkflowRun, String, String) {
        let wr = self
            .workflows
            .start(StartRequest {
                workflow_id: flow.into(),
                project_id: self.project.clone(),
                task: "The tray icon is invisible in dark mode.".into(),
                trigger: WorkflowTrigger::Manual,
                origin_run_id: None,
                test,
            })
            .await
            .unwrap();
        let wr = self.until(&wr.id, waits_on(WaitingOn::Check)).await;
        let branch = newest(&wr, "dev").handoff.as_ref().unwrap().branches[0]
            .branch
            .clone();
        let tip = git_out(&self.repo, &["rev-parse", &format!("refs/heads/{branch}")]);
        (wr, branch, tip)
    }

    /// Holds every job at a gate, approves the check the workflow run `id` waits on, and
    /// waits until the deploy's job is held. Gives the deploy's step run id and the gate.
    async fn approved_and_held(&self, id: &str) -> (String, Arc<tokio::sync::Semaphore>) {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        self.workflows.hold_jobs(Some(gate.clone()));
        let wr = self.approve(id, None).await.unwrap();
        let step_run = newest(&wr, "ship").clone();
        assert_eq!(step_run.status, StepRunStatus::Running);
        assert!(self.workflows.has_job(&step_run.id));
        (step_run.id, gate)
    }

    /// Moves `branch` on by one empty commit, made outside any worktree, as someone working
    /// on it after the approval would. Gives the new tip.
    fn commit_after(&self, branch: &str) -> String {
        let tip = git_out(&self.repo, &["rev-parse", &format!("refs/heads/{branch}")]);
        let tree = format!("{tip}^{{tree}}");
        let later = git_out(
            &self.repo,
            &["commit-tree", &tree, "-p", &tip, "-m", "Later."],
        );
        git_out(
            &self.repo,
            &["update-ref", &format!("refs/heads/{branch}"), &later],
        );
        later
    }
}

/// What a push deploy did to `branch` at `commit`.
fn pushed(branch: &str, commit: &str) -> DeployedBranch {
    DeployedBranch {
        branch: branch.into(),
        commit: commit.into(),
        pushed: true,
        pr_url: None,
        auto_merge: false,
    }
}

#[tokio::test]
async fn a_push_deploy_lands_the_commit_the_check_approved_on_the_remote() {
    let w = world().await;
    w.shipping_to_origin("ok");
    let flow = w.ship_flow(DeployMode::Push).await;
    let (wr, branch, tip) = w.at_the_check(&flow, false).await;
    assert_eq!(w.remote_tip(&branch), None);
    w.approve(&wr.id, Some("Ship it.")).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    // The approval recorded the tip it showed, and the deploy pushed exactly that.
    assert_eq!(
        newest(&wr, "check").decision,
        Some(json!({
            "decision": "approved",
            "note": "Ship it.",
            "branches": [{"branch": branch, "commit": tip}],
        }))
    );
    assert_eq!(w.remote_tip(&branch), Some(tip.clone()));
    let ship = newest(&wr, "ship");
    assert_eq!(ship.status, StepRunStatus::Succeeded);
    assert_eq!(
        ship.deploy_result,
        Some(DeployResult {
            branches: vec![pushed(&branch, &tip)],
            ready: false,
            error: None,
        })
    );
    // A deploy that succeeded records no error, not even a null one.
    let result = serde_json::to_value(&ship.deploy_result).unwrap();
    assert!(result.get("error").is_none(), "{result}");
    assert!(w.gh_calls().is_empty());
}

#[tokio::test]
async fn a_pr_merge_deploy_opens_a_pull_request_and_sets_it_to_merge() {
    let w = world().await;
    w.shipping_to_origin("ok");
    let flow = w
        .ship_flow(DeployMode::PrMerge {
            merge_method: MergeMethod::default(),
        })
        .await;
    let (wr, branch, tip) = w.at_the_check(&flow, false).await;
    w.approve(&wr.id, None).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(w.remote_tip(&branch), Some(tip.clone()));
    let url = "https://github.com/o/r/pull/1";
    // Against the default branch, titled with the task, with the approved summary as the body,
    // then auto-merge by squashing.
    let head = format!("--head={branch}");
    let calls: Vec<Vec<&str>> = vec![
        vec!["--version"],
        vec![
            "pr",
            "list",
            &head,
            "--base=main",
            "--state=open",
            "--json=url,headRefName,baseRefName,isCrossRepository",
        ],
        vec![
            "pr",
            "create",
            &head,
            "--base=main",
            "--title=The tray icon is invisible in dark mode.",
            "--body-file=-",
        ],
        vec!["pr", "merge", url, "--auto", "--squash"],
    ];
    assert_eq!(w.gh_calls(), calls);
    assert_eq!(
        newest(&wr, "ship").deploy_result,
        Some(DeployResult {
            branches: vec![DeployedBranch {
                pr_url: Some(url.into()),
                auto_merge: true,
                ..pushed(&branch, &tip)
            }],
            ready: false,
            error: None,
        })
    );
}

#[tokio::test]
async fn a_test_run_pushes_nothing_and_lists_the_branch_as_ready() {
    let w = world().await;
    w.shipping_to_origin("ok");
    let flow = w
        .ship_flow(DeployMode::PrMerge {
            merge_method: MergeMethod::Rebase,
        })
        .await;
    let (wr, branch, tip) = w.at_the_check(&flow, true).await;
    w.approve(&wr.id, None).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(
        newest(&wr, "ship").deploy_result,
        Some(DeployResult {
            branches: vec![DeployedBranch {
                pushed: false,
                ..pushed(&branch, &tip)
            }],
            ready: true,
            error: None,
        })
    );
    assert_eq!(w.remote_tip(&branch), None);
    assert!(w.gh_calls().is_empty());
}

#[tokio::test]
async fn open_folder_finds_a_project_s_repo_and_a_ready_branch_s_worktree_by_id_only() {
    let w = world().await;
    w.shipping_to_origin("ok");
    let flow = w.ship_flow(DeployMode::Push).await;
    let no_ready =
        |id: &str, branch: &str| format!("workflow run {id} has no ready branch {branch}");

    assert_eq!(
        w.workflows.project_folder(&w.project).await.unwrap(),
        w.repo
    );
    assert!(matches!(
        w.workflows.project_folder("nope").await,
        Err(Error::NotFound(m)) if m == "project nope"
    ));

    // A test run's deploy lists the branch as ready; until then, nothing is.
    let (wr, branch, _) = w.at_the_check(&flow, true).await;
    assert!(matches!(
        w.workflows.branch_folder(&wr.id, &branch).await,
        Err(Error::Invalid(m)) if m == no_ready(&wr.id, &branch)
    ));
    w.approve(&wr.id, None).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    let worktree =
        PathBuf::from(&newest(&wr, "dev").handoff.as_ref().unwrap().branches[0].worktree_path);
    assert!(worktree.is_dir());
    assert_eq!(
        w.workflows.branch_folder(&wr.id, &branch).await.unwrap(),
        worktree
    );
    // Only a branch the run lists as ready: never another ref or a path.
    for other in ["main", "HEAD", "..", worktree.to_str().unwrap()] {
        assert!(matches!(
            w.workflows.branch_folder(&wr.id, other).await,
            Err(Error::Invalid(m)) if m == no_ready(&wr.id, other)
        ));
    }

    // A pushed branch is not ready: its folder is not offered.
    let (pushed, pushed_branch, _) = w.at_the_check(&flow, false).await;
    w.approve(&pushed.id, None).await.unwrap();
    let pushed = w
        .until(&pushed.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert!(
        !newest(&pushed, "ship")
            .deploy_result
            .as_ref()
            .unwrap()
            .ready
    );
    assert!(matches!(
        w.workflows.branch_folder(&pushed.id, &pushed_branch).await,
        Err(Error::Invalid(m)) if m == no_ready(&pushed.id, &pushed_branch)
    ));

    // A folder that is gone is said to be gone.
    std::fs::remove_dir_all(&worktree).unwrap();
    assert!(matches!(
        w.workflows.branch_folder(&wr.id, &branch).await,
        Err(Error::NotFound(m)) if m == format!("folder {} no longer exists", worktree.display())
    ));
}

#[tokio::test]
async fn a_branch_changed_after_the_approval_fails_the_deploy_until_it_is_put_back() {
    let w = world().await;
    w.shipping_to_origin("ok");
    let flow = w.ship_flow(DeployMode::Push).await;
    let (wr, branch, tip) = w.at_the_check(&flow, false).await;
    let (_, gate) = w.approved_and_held(&wr.id).await;
    w.commit_after(&branch);
    w.workflows.hold_jobs(None);
    gate.add_permits(1);
    let failed = w.until(&wr.id, waits_on(WaitingOn::StepFailed)).await;
    assert_eq!(
        failed.error.as_deref(),
        Some("Ship: The branch changed after it was approved.")
    );
    // Refused before any push: the branch is recorded as not pushed, with the error.
    assert_eq!(
        serde_json::to_value(&newest(&failed, "ship").deploy_result).unwrap(),
        json!({
            "branches": [{
                "branch": branch,
                "commit": tip,
                "pushed": false,
                "pr_url": null,
                "auto_merge": false,
            }],
            "ready": false,
            "error": "The branch changed after it was approved.",
        })
    );
    assert_eq!(w.remote_tip(&branch), None);
    // Put back where it was approved, a retry deploys it.
    git_out(
        &w.repo,
        &["update-ref", &format!("refs/heads/{branch}"), &tip],
    );
    w.workflows.retry_step(&wr.id).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(w.remote_tip(&branch), Some(tip));
    assert_eq!(
        attempts(&wr),
        [
            ("dev", 1, StepRunStatus::Succeeded),
            ("check", 1, StepRunStatus::Succeeded),
            ("ship", 1, StepRunStatus::Failed),
            ("ship", 2, StepRunStatus::Succeeded),
        ]
    );
}

#[tokio::test]
async fn a_deploy_refuses_a_branch_that_holds_a_secret() {
    let w = world().await;
    w.shipping_to_origin("ok");
    w.engine
        .set(&w.dev.spec.name, vec![Act::Write(".env", "TOKEN=abc")]);
    let flow = w
        .workflow(
            "Ship",
            vec![
                agent("dev", "Fix it", &w.dev, "Fix the bug."),
                check("check"),
                ship(DeployMode::Pr),
            ],
        )
        .await;
    let (wr, branch, _) = w.at_the_check(&flow, false).await;
    w.approve(&wr.id, None).await.unwrap();
    let wr = w.until(&wr.id, waits_on(WaitingOn::StepFailed)).await;
    assert_eq!(
        wr.error,
        Some(format!(
            "Ship: Not pushed: {branch} has commits that hold files that look like secrets or are \
             over 10 MB: .env. Rewrite the branch so none of the commits it would push holds \
             them, then retry the step."
        ))
    );
    // Refused before any push: the branch is recorded as not pushed, with the error.
    let result = serde_json::to_value(&newest(&wr, "ship").deploy_result).unwrap();
    assert_eq!(result["branches"][0]["branch"], json!(branch));
    assert_eq!(result["branches"][0]["pushed"], json!(false));
    assert_eq!(
        result["error"].as_str(),
        wr.error.as_deref().and_then(|e| e.strip_prefix("Ship: "))
    );
    assert_eq!(w.remote_tip(&branch), None);
    assert!(w.gh_calls().is_empty());
}

#[tokio::test]
async fn a_failed_gh_fails_the_deploy_with_its_message_and_keeps_the_push() {
    let w = world().await;
    w.shipping_to_origin("fail-merge");
    let flow = w
        .ship_flow(DeployMode::PrMerge {
            merge_method: MergeMethod::Merge,
        })
        .await;
    let (wr, branch, tip) = w.at_the_check(&flow, false).await;
    w.approve(&wr.id, None).await.unwrap();
    let wr = w.until(&wr.id, waits_on(WaitingOn::StepFailed)).await;
    assert_eq!(
        wr.error.as_deref(),
        Some(
            "Ship: gh pr merge failed: GraphQL: Pull request Auto merge is not allowed for this \
             repository (enablePullRequestAutoMerge)"
        )
    );
    assert_eq!(newest(&wr, "ship").status, StepRunStatus::Failed);
    // Nothing already done is undone, and what was done is kept with the error: the branch
    // pushed and the pull request, to merge by hand.
    assert_eq!(w.remote_tip(&branch), Some(tip.clone()));
    assert_eq!(w.gh_calls().last().unwrap()[1], "merge");
    assert_eq!(
        serde_json::to_value(&newest(&wr, "ship").deploy_result).unwrap(),
        json!({
            "branches": [{
                "branch": branch,
                "commit": tip,
                "pushed": true,
                "pr_url": "https://github.com/o/r/pull/1",
                "auto_merge": false,
            }],
            "ready": false,
            "error": "gh pr merge failed: GraphQL: Pull request Auto merge is not allowed for \
                      this repository (enablePullRequestAutoMerge)",
        })
    );
}

#[tokio::test]
async fn a_deploy_cut_off_by_a_restart_fails_interrupted_and_is_not_pushed_until_retried() {
    let w = world().await;
    w.shipping_to_origin("ok");
    let flow = w.ship_flow(DeployMode::Push).await;
    let (wr, branch, tip) = w.at_the_check(&flow, false).await;
    let (step_run, gate) = w.approved_and_held(&wr.id).await;
    // Dots stops while the deploy waits: its loop ends and its job is dropped unwritten.
    w.stop_loop.cancel();
    gate.close();
    w.job_ended(&step_run).await;
    let fresh = Workflows::new(
        w.store.clone(),
        new_bus(),
        w.runner.clone(),
        Arc::new(SystemClock),
    );
    fresh.pass().await.unwrap();
    let failed = w.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(failed.waiting_on, Some(WaitingOn::StepFailed));
    assert_eq!(failed.error.as_deref(), Some("Ship: interrupted"));
    // Further passes leave it waiting: nothing is pushed without the person.
    fresh.pass().await.unwrap();
    assert_eq!(w.remote_tip(&branch), None);
    fresh.retry_step(&wr.id).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(w.remote_tip(&branch), Some(tip));
    assert_eq!(newest(&wr, "ship").attempt, 2);
}

#[tokio::test]
async fn a_stop_during_a_held_deploy_wins_and_nothing_is_pushed() {
    let w = world().await;
    w.shipping_to_origin("ok");
    let flow = w.ship_flow(DeployMode::Push).await;
    let (wr, branch, _) = w.at_the_check(&flow, false).await;
    let (step_run, gate) = w.approved_and_held(&wr.id).await;
    w.workflows.stop(&wr.id).await.unwrap();
    gate.add_permits(1);
    w.job_ended(&step_run).await;
    w.workflows.pass().await.unwrap();
    let wr = w.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(wr.status, WorkflowRunStatus::Cancelled);
    let ship = newest(&wr, "ship");
    assert_eq!(ship.status, StepRunStatus::Cancelled);
    assert_eq!(ship.deploy_result, None);
    assert_eq!(w.remote_tip(&branch), None);
}

#[tokio::test]
async fn a_member_deploy_runs_the_members_step_and_a_test_run_lists_the_branch_instead() {
    let w = world().await;
    w.engine.set(
        &w.helper.spec.name,
        vec![Act::Write("release.txt", "Tagged the release.")],
    );
    let flow = w
        .ship_flow(DeployMode::Member {
            member: w.helper.id.clone(),
            job: "Tag a release.".into(),
        })
        .await;
    let (wr, branch, tip) = w.at_the_check(&flow, false).await;
    w.approve(&wr.id, None).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    // The member's run is the step's: its job, its place, and the approved work to start from.
    let prompts = w.engine.prompts_of("helper");
    assert_eq!(prompts.len(), 1);
    for part in [
        "- Step: Ship (3 of 3)",
        "### Your job in this step\n\nTag a release.",
        "#### Approved\n\nFixed the icon.",
    ] {
        assert!(
            prompts[0].contains(part),
            "missing {part:?} in:\n{}",
            prompts[0]
        );
    }
    let ship = newest(&wr, "ship");
    let runs = w.runs_of(&ship.id).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].dot_id, w.helper.id);
    assert_eq!(ship.deploy_result, None);
    let handoff = ship.handoff.as_ref().unwrap();
    assert_eq!(handoff.summary, "Tagged the release.");
    let release = &handoff.branches[0].branch;
    assert_eq!(
        git_out(&w.repo, &["show", &format!("{release}:fix.txt")]),
        "Fixed the icon."
    );
    assert_eq!(
        git_out(&w.repo, &["merge-base", "--is-ancestor", &tip, release]),
        ""
    );
    assert_ne!(release, &branch);

    // A test run starts no one: the deploy lists the branch as ready.
    let (wr, branch, tip) = w.at_the_check(&flow, true).await;
    w.approve(&wr.id, None).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(w.engine.prompts_of("helper").len(), 1);
    assert_eq!(
        newest(&wr, "ship").deploy_result,
        Some(DeployResult {
            branches: vec![DeployedBranch {
                pushed: false,
                ..pushed(&branch, &tip)
            }],
            ready: true,
            error: None,
        })
    );
}

#[tokio::test]
async fn a_deploy_after_a_delegate_step_pushes_the_members_branches() {
    let w = world().await;
    w.shipping_to_origin("ok");
    w.engine.set(
        &w.helper.spec.name,
        vec![
            Act::Start(
                vec![("dev", "Part one."), ("dev", "Part two.")],
                Box::new(Act::Finish("Started two.")),
            ),
            Act::Finish("Both are in."),
        ],
    );
    w.engine.set(
        &w.dev.spec.name,
        vec![
            Act::Write("one.txt", "Part one done."),
            Act::Write("two.txt", "Part two done."),
        ],
    );
    let flow = w
        .workflow(
            "Split",
            vec![
                delegating("split", "Split it", &w.helper, &[&w.dev], 2, 2),
                check("check"),
                ship(DeployMode::Push),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, waits_on(WaitingOn::Check)).await;
    // The lead merged nothing, so the check shows each member's branch (section 6.2).
    let shown: Vec<String> = newest(&wr, "split")
        .handoff
        .as_ref()
        .unwrap()
        .branches
        .iter()
        .map(|b| b.branch.clone())
        .collect();
    assert_eq!(shown.len(), 2);
    w.approve(&wr.id, None).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    let result = newest(&wr, "ship").deploy_result.clone().unwrap();
    let tips: Vec<String> = shown
        .iter()
        .map(|b| git_out(&w.repo, &["rev-parse", &format!("refs/heads/{b}")]))
        .collect();
    assert_eq!(
        result.branches,
        [pushed(&shown[0], &tips[0]), pushed(&shown[1], &tips[1])]
    );
    for (b, tip) in shown.iter().zip(&tips) {
        assert_eq!(w.remote_tip(b).as_ref(), Some(tip));
    }
}

#[tokio::test]
async fn a_member_deploy_starts_from_the_approved_commit() {
    let w = world().await;
    w.engine.set(
        &w.dev.spec.name,
        vec![
            Act::Write("a.txt", "Approved work."),
            Act::Write("b.txt", "Work nobody approved."),
        ],
    );
    w.engine.set(
        &w.helper.spec.name,
        vec![Act::Write("release.txt", "Tagged the release.")],
    );
    let flow = w
        .workflow(
            "Late",
            vec![
                agent("dev", "Fix it", &w.dev, "Fix the bug."),
                check("check"),
                agent("more", "More", &w.dev, "Do more."),
                ship(DeployMode::Member {
                    member: w.helper.id.clone(),
                    job: "Tag a release.".into(),
                }),
            ],
        )
        .await;
    let (wr, _, tip) = w.at_the_check(&flow, false).await;
    w.approve(&wr.id, None).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    // The later step's branch is not what the member starts from: the approved commit is.
    let runs = w.runs_of(&newest(&wr, "ship").id).await;
    assert_eq!(runs.len(), 1);
    let start = runs[0].payload.as_ref().unwrap().pointer("/workflow/start");
    assert_eq!(start, Some(&json!(tip)));
    let release = &newest(&wr, "ship").handoff.as_ref().unwrap().branches[0].branch;
    let files = git_out(&w.repo, &["ls-tree", "-r", "--name-only", release]);
    assert!(
        files.contains("a.txt") && !files.contains("b.txt"),
        "{files}"
    );
}

/// A run of `flow` stopped at its deploy, which waits on the budget (the dev's run spent it),
/// and the dev's branch.
async fn member_deploy_on_budget(budget_w: &World, flow: &str) -> (WorkflowRun, String) {
    let (wr, branch, _) = budget_w.at_the_check(flow, false).await;
    budget_w.approve(&wr.id, None).await.unwrap();
    let held = budget_w.until(&wr.id, waits_on(WaitingOn::Budget)).await;
    assert!(budget_w.runs_of(&newest(&held, "ship").id).await.is_empty());
    (held, branch)
}

async fn budget_world() -> World {
    let w = world_with(
        Arc::new(SystemClock),
        Budget {
            max_runs: 1,
            ..Budget::default()
        },
    )
    .await;
    w.engine.set(
        &w.helper.spec.name,
        vec![Act::Write("release.txt", "Tagged the release.")],
    );
    w
}

#[tokio::test]
async fn a_member_deploy_is_refused_when_the_branch_moved() {
    let w = budget_world().await;
    let flow = w
        .ship_flow(DeployMode::Member {
            member: w.helper.id.clone(),
            job: "Tag a release.".into(),
        })
        .await;
    let (held, branch) = member_deploy_on_budget(&w, &flow).await;
    w.commit_after(&branch);
    w.workflows.add_budget(&held.id).await.unwrap();
    let failed = w.until(&held.id, waits_on(WaitingOn::StepFailed)).await;
    assert_eq!(
        failed.error.as_deref(),
        Some("Ship: The branch changed after it was approved.")
    );
    assert!(w.runs_of(&newest(&failed, "ship").id).await.is_empty());
    assert_eq!(w.engine.prompts_of("helper").len(), 0);
}

#[tokio::test]
async fn a_member_deploy_waiting_on_budget_still_checks_the_tip() {
    let w = budget_world().await;
    let flow = w
        .ship_flow(DeployMode::Member {
            member: w.helper.id.clone(),
            job: "Tag a release.".into(),
        })
        .await;
    let (held, branch) = member_deploy_on_budget(&w, &flow).await;
    assert_eq!(held.error.as_deref(), Some("Budget reached: runs, 1 of 1"));
    let tip = git_out(&w.repo, &["rev-parse", &format!("refs/heads/{branch}")]);
    w.commit_after(&branch);
    // The budget comes only after the branch moved, and the member never starts.
    w.workflows.add_budget(&held.id).await.unwrap();
    let failed = w.until(&held.id, waits_on(WaitingOn::StepFailed)).await;
    assert_eq!(newest(&failed, "ship").status, StepRunStatus::Failed);
    assert!(w.runs_of(&newest(&failed, "ship").id).await.is_empty());
    // Put back, a retry starts the member.
    git_out(
        &w.repo,
        &["update-ref", &format!("refs/heads/{branch}"), &tip],
    );
    w.workflows.retry_step(&failed.id).await.unwrap();
    let done = w
        .until(&failed.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(newest(&done, "ship").attempt, 2);
}

#[tokio::test]
async fn a_deploy_with_no_recorded_approval_fails_without_pushing() {
    let w = world().await;
    w.shipping_to_origin("ok");
    let flow = w.ship_flow(DeployMode::Push).await;
    let (wr, branch, _) = w.at_the_check(&flow, false).await;
    let (_, gate) = w.approved_and_held(&wr.id).await;
    // An approval made before Dots recorded the branches.
    let wr = w.store.get_workflow_run(&wr.id).await.unwrap();
    w.store
        .set_step_run(
            &newest(&wr, "check").id,
            &dots_core::model::StepRunChange {
                decision: Some(Some(json!({"decision": "approved", "note": null}))),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    gate.add_permits(1);
    let failed = w.until(&wr.id, waits_on(WaitingOn::StepFailed)).await;
    assert_eq!(
        failed.error.as_deref(),
        Some("Ship: the check before it recorded no approved branches")
    );
    assert_eq!(w.remote_tip(&branch), None);
}

#[tokio::test]
async fn a_pr_deploy_needs_a_default_branch_and_a_push_deploy_does_not() {
    // A detached repo with no remote HEAD and no default branch set names no branch.
    let w = world().await;
    w.shipping_to_origin("ok");
    let pr = w.ship_flow(DeployMode::Pr).await;
    git_out(&w.repo, &["checkout", "-q", "--detach"]);
    let (wr, branch, _) = w.at_the_check(&pr, false).await;
    w.approve(&wr.id, None).await.unwrap();
    let failed = w.until(&wr.id, waits_on(WaitingOn::StepFailed)).await;
    assert_eq!(
        failed.error.as_deref(),
        Some("Ship: the project's default branch could not be found; set it on the project")
    );
    assert_eq!(w.remote_tip(&branch), None);
    let push = w
        .workflow(
            "Ship by push",
            vec![
                agent("dev", "Fix it", &w.dev, "Fix the bug."),
                check("check"),
                ship(DeployMode::Push),
            ],
        )
        .await;
    w.engine.set(
        &w.dev.spec.name,
        vec![Act::Write("fix.txt", "Fixed the icon.")],
    );
    let (wr, branch, tip) = w.at_the_check(&push, false).await;
    w.approve(&wr.id, None).await.unwrap();
    w.until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(w.remote_tip(&branch), Some(tip));
}

/// The flow "Late": A (the dev) writes `a.txt`, the person checks, B (the dev) writes more,
/// then a member deploy by the helper, then `steps_after`.
async fn late_member_deploy(w: &World, steps_after: Vec<Step>) -> String {
    let mut steps = vec![
        agent("dev", "Fix it", &w.dev, "Fix the bug."),
        check("check"),
        agent("more", "More", &w.dev, "Do more."),
        ship(DeployMode::Member {
            member: w.helper.id.clone(),
            job: "Tag a release.".into(),
        }),
    ];
    steps.extend(steps_after);
    w.workflow("Late", steps).await
}

#[tokio::test]
async fn a_member_deploy_run_again_after_a_send_back_starts_from_the_approved_commit() {
    let w = world().await;
    w.engine.set(
        &w.dev.spec.name,
        vec![
            Act::Write("a.txt", "Approved work."),
            Act::Write("b.txt", "Work nobody approved."),
            Act::Write("b2.txt", "More work nobody approved."),
        ],
    );
    w.engine.set(
        &w.helper.spec.name,
        vec![
            Act::Write("release.txt", "Tagged the release."),
            Act::SendBack(
                "more",
                "Do it again.",
                Box::new(Act::Finish("Sent it back.")),
            ),
            Act::Write("release2.txt", "Tagged again."),
            Act::Finish("Reviewed."),
        ],
    );
    let flow = late_member_deploy(
        &w,
        vec![sending_back(
            "review",
            "Review",
            &w.helper,
            "Review it.",
            &["more"],
        )],
    )
    .await;
    let (wr, branch, tip) = w.at_the_check(&flow, false).await;
    w.approve(&wr.id, None).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    let ships: Vec<_> = wr.steps.iter().filter(|s| s.step_id == "ship").collect();
    assert_eq!(ships.len(), 2);
    let b_branch = newest(&wr, "more").handoff.as_ref().unwrap().branches[0]
        .branch
        .clone();
    let prompts = w.engine.prompts_of("helper");
    for (i, ship) in ships.iter().enumerate() {
        let runs = w.runs_of(&ship.id).await;
        assert_eq!(runs.len(), 1);
        let start = runs[0].payload.as_ref().unwrap().pointer("/workflow/start");
        assert_eq!(start, Some(&json!(tip)), "attempt {}", i + 1);
        // The prompts of the two attempts (the review's run is between them) name only the
        // approved branch and its commit, not the later step's branch.
        let prompt = &prompts[if i == 0 { 0 } else { 2 }];
        assert!(
            prompt.contains(&format!("{branch} (commit {tip})")),
            "{prompt}"
        );
        assert!(!prompt.contains(&b_branch), "{prompt}");
        assert!(prompt.contains("### Your job in this step\n\nTag a release."));
    }
}

#[tokio::test]
async fn a_member_deploy_after_a_later_step_is_refused_if_the_approved_branch_moved() {
    let w = world().await;
    w.engine.set(
        &w.dev.spec.name,
        vec![
            Act::Write("a.txt", "Approved work."),
            Act::Await("b.txt", "B done."),
        ],
    );
    let flow = late_member_deploy(&w, vec![]).await;
    let (wr, branch, _) = w.at_the_check(&flow, false).await;
    w.approve(&wr.id, None).await.unwrap();
    // While the later step runs, a commit lands on the approved branch.
    w.commit_after(&branch);
    w.engine.held.add_permits(1);
    let failed = w.until(&wr.id, waits_on(WaitingOn::StepFailed)).await;
    assert_eq!(
        failed.error.as_deref(),
        Some("Ship: The branch changed after it was approved.")
    );
    assert!(w.runs_of(&newest(&failed, "ship").id).await.is_empty());
    assert_eq!(w.engine.prompts_of("helper").len(), 0);
}

#[tokio::test]
async fn a_check_after_a_step_that_changed_nothing_shows_and_ships_the_earlier_branch() {
    let w = world().await;
    w.shipping_to_origin("ok");
    w.engine.set(
        &w.dev.spec.name,
        vec![Act::Write("fix.txt", "Fixed the icon.")],
    );
    w.engine.set(
        &w.helper.spec.name,
        vec![Act::Finish("Read it: it is right.")],
    );
    let flow = w
        .workflow(
            "Reviewed",
            vec![
                agent("dev", "Fix it", &w.dev, "Fix the bug."),
                agent("review", "Review it", &w.helper, "Review the fix."),
                check("check"),
                ship(DeployMode::Push),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, waits_on(WaitingOn::Check)).await;
    let built = newest(&wr, "dev").handoff.clone().unwrap().branches;
    let branch = built[0].branch.clone();
    let tip = git_out(&w.repo, &["rev-parse", &format!("refs/heads/{branch}")]);
    // The reviewer changed nothing: its branch is gone with its worktree.
    assert!(newest(&wr, "review")
        .handoff
        .as_ref()
        .unwrap()
        .branches
        .is_empty());
    w.approve(&wr.id, None).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    // The check showed the reviewer's summary with the builder's branch, and approved its tip.
    let check = newest(&wr, "check");
    let shown = check.handoff.as_ref().unwrap();
    assert_eq!(shown.summary, "Read it: it is right.");
    assert_eq!(shown.branches, built);
    assert_eq!(
        check.decision.as_ref().unwrap()["branches"],
        json!([{"branch": branch, "commit": tip}])
    );
    // The deploy pushed it.
    assert_eq!(w.remote_tip(&branch), Some(tip.clone()));
    assert_eq!(
        newest(&wr, "ship").deploy_result.as_ref().unwrap().branches,
        [pushed(&branch, &tip)]
    );
}

#[tokio::test]
async fn an_outcome_step_sent_back_is_told_to_report_again_and_the_branch_takes_the_new_path() {
    let w = world().await;
    w.engine.set(&w.dev.spec.name, vec![Act::Gate, Act::Gate]);
    let flow = w
        .workflow(
            "Triage",
            vec![
                reporting("triage", "Triage", &w.dev, &["fixed", "needs_info"]),
                branch(
                    "route",
                    BranchOn::Outcome,
                    vec![
                        ("fixed", vec![check("check")]),
                        (
                            "needs_info",
                            vec![agent("ask", "Ask", &w.helper, "Ask the person.")],
                        ),
                    ],
                    vec![],
                ),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    let first = w.until(&wr.id, |wr| !wr.steps.is_empty()).await.steps[0].clone();
    w.store
        .set_step_outcome(&first.id, Some("fixed"))
        .await
        .unwrap();
    w.engine.gate.notify_one();
    let wr = w.until(&wr.id, at_check(1)).await;
    assert_eq!(chose(&wr, "route").as_deref(), Some("fixed"));
    w.send_back(&wr.id, "Ask the person first.").await.unwrap();
    let again = w
        .until(&wr.id, |wr| {
            wr.steps
                .iter()
                .any(|s| s.step_id == "triage" && s.attempt == 2)
        })
        .await;
    let again = newest(&again, "triage").clone();
    assert_eq!(again.outcome, None);
    // The continued run is told its earlier outcome no longer counts, and reports another.
    w.store
        .set_step_outcome(&again.id, Some("needs_info"))
        .await
        .unwrap();
    w.engine.gate.notify_one();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(
        ran(&wr),
        ["triage", "route", "check", "triage", "route", "ask"]
    );
    assert_eq!(chose(&wr, "route").as_deref(), Some("needs_info"));
    assert_eq!(
        w.engine.prompts_of("dev")[1],
        "The person sent this back: Ask the person first.\n\nReport the outcome again with \
         `mcp__dots__report_outcome`, one of: \"fixed\", \"needs_info\": the earlier one no \
         longer counts."
    );
}

#[tokio::test]
async fn a_send_back_to_a_name_several_steps_share_is_refused_naming_them() {
    let w = world().await;
    w.engine.set(&w.dev.spec.name, vec![Act::Gate]);
    let flow = w
        .workflow("Fix", vec![agent("dev", "Fix it", &w.dev, "Fix.")])
        .await;
    let wr = w.start(&flow).await;
    let step_run = w.until(&wr.id, |wr| !wr.steps.is_empty()).await.steps[0].clone();
    let target = |id: &str, name: &str| SendBackTarget {
        step_id: id.into(),
        name: name.into(),
    };
    let tools = StepTools {
        step_run_id: step_run.id.clone(),
        step_id: "dev".into(),
        outcomes: vec![],
        send_back_to: vec![
            target("a", "Build it"),
            target("b", "Review"),
            target("c", "build IT"),
        ],
        delegate: None,
    };
    let refused = w
        .workflows
        .call_send_back(
            "run",
            &tools,
            &json!({"to_step": "BUILD IT", "note": "Again."}),
        )
        .await;
    assert_eq!(
        refused,
        Err(
            "\"BUILD IT\" names more than one step you can send work back to: \"Build it\", \
             \"build IT\". Nothing was recorded."
                .to_string()
        )
    );
    let after = w.store.get_step_run(&step_run.id).await.unwrap();
    assert_eq!(after.send_back, None);
    w.workflows.stop(&wr.id).await.unwrap();
}

#[tokio::test]
async fn a_hand_off_whose_steps_runs_changed_while_it_was_built_writes_nothing() {
    let w = world().await;
    w.engine.set(
        &w.dev.spec.name,
        vec![
            Act::Write("one.txt", "First."),
            Act::Gate,
            Act::Finish("Second."),
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
    let (held, step, gate) = w.held_hand_off(&flow).await;
    // The person's message joins the step's thread while its hand-off is built.
    let run = w.runs_of(&step).await.remove(0);
    let mut new = NewRun::new(&w.dev.id, TriggerKind::Chat);
    new.payload = Some(json!({"message": "And the tooltip?"}));
    new.parent_run_id = Some(run.id.clone());
    new.root_run_id = Some(run.root_run_id.clone());
    new.session_id = run.session_id.clone();
    new.workspace_path = run.workspace_path.clone();
    new.branch = run.branch.clone();
    new.base_commit = run.base_commit.clone();
    let reply = w.runner.enqueue(new).await.unwrap();
    assert_eq!(reply.workflow_step_run_id.as_deref(), Some(step.as_str()));
    w.workflows.hold_jobs(None);
    gate.add_permits(1);
    w.job_ended(&step).await;
    // The job read runs that changed since: it wrote nothing, and the step is still open.
    let wr = w.store.get_workflow_run(&held.id).await.unwrap();
    assert_eq!(ran(&wr), ["one"]);
    assert_eq!(wr.steps[0].status, StepRunStatus::Running);
    assert!(wr.steps[0].handoff.is_none());
    // Once the reply has ended, the hand-off is built again with its work.
    w.engine.gate.notify_one();
    let wr = w
        .until(&held.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(ran(&wr), ["one", "two"]);
    assert_eq!(wr.steps[0].handoff.as_ref().unwrap().summary, "through");
    assert!(wr.steps.iter().all(|s| s.attempt == 1));
}

/// Starts a run of `flow`, whose first step is a check, and leaves it as a move cut off after
/// the check would: the check succeeded with `decision` and a step run of `next` made, but the
/// workflow run not yet moved to it. Gives the workflow run's id and that step run's id.
async fn cut_off_after_the_check(
    w: &World,
    flow: &str,
    next: &str,
    decision: Option<serde_json::Value>,
) -> (String, String) {
    let wr = w.start(flow).await;
    let wr = w.until(&wr.id, waits_on(WaitingOn::Check)).await;
    w.store
        .set_step_run(
            &wr.steps[0].id,
            &StepRunChange {
                status: Some(StepRunStatus::Succeeded),
                decision: Some(decision),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let orphan = w
        .store
        .create_step_run(&wr.id, next, 1, StepRunStatus::Running)
        .await
        .unwrap();
    w.store
        .set_workflow_run(
            &wr.id,
            &WorkflowRunChange {
                status: Some(WorkflowRunStatus::Running),
                waiting_on: Some(None),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    w.workflows.pass().await.unwrap();
    (wr.id, orphan.id)
}

#[tokio::test]
async fn a_step_run_cut_off_before_the_workflow_run_moved_to_it_is_used_not_left_running() {
    let w = world().await;
    let done = |wr: &WorkflowRun| wr.status == WorkflowRunStatus::Succeeded;
    // An agent step.
    let flow = w
        .workflow(
            "Agent",
            vec![check("check"), agent("dev", "Fix it", &w.dev, "Fix.")],
        )
        .await;
    let (id, orphan) = cut_off_after_the_check(&w, &flow, "dev", None).await;
    let wr = w.until(&id, done).await;
    assert_eq!(
        attempts(&wr),
        [
            ("check", 1, StepRunStatus::Succeeded),
            ("dev", 1, StepRunStatus::Succeeded),
        ]
    );
    assert_eq!(newest(&wr, "dev").id, orphan);
    assert_eq!(w.runs_of(&orphan).await.len(), 1);

    // A branch.
    let flow = w
        .workflow(
            "Branch",
            vec![
                check("check"),
                branch(
                    "pick",
                    BranchOn::Project,
                    vec![("Sage", vec![agent("dots", "Sage", &w.dev, "Work.")])],
                    vec![],
                ),
            ],
        )
        .await;
    let (id, orphan) = cut_off_after_the_check(&w, &flow, "pick", None).await;
    let wr = w.until(&id, done).await;
    assert_eq!(ran(&wr), ["check", "pick", "dots"]);
    assert_eq!(newest(&wr, "pick").id, orphan);
    assert_eq!(chose(&wr, "pick").as_deref(), Some("Sage"));

    // A deploy.
    let flow = w
        .workflow("Deploy", vec![check("check"), ship(DeployMode::Branch)])
        .await;
    let approved = json!({"decision": "approved", "note": null, "branches": []});
    let (id, orphan) = cut_off_after_the_check(&w, &flow, "ship", Some(approved)).await;
    let wr = w.until(&id, done).await;
    assert_eq!(
        attempts(&wr),
        [
            ("check", 1, StepRunStatus::Succeeded),
            ("ship", 1, StepRunStatus::Succeeded),
        ]
    );
    assert_eq!(newest(&wr, "ship").id, orphan);
}

impl World {
    /// Moves `branch` on by a commit that adds the file `path` with `text`, made outside any
    /// worktree (in an index of its own), as someone working on it would. Gives the new tip.
    fn commit_file(&self, branch: &str, path: &str, text: &str) -> String {
        let tip = git_out(&self.repo, &["rev-parse", &format!("refs/heads/{branch}")]);
        let file = self.dir.path().join("blob.txt");
        std::fs::write(&file, text).unwrap();
        let blob = git_out(&self.repo, &["hash-object", "-w", &file.to_string_lossy()]);
        let index = self.dir.path().join("index.tmp");
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&self.repo)
                .env("GIT_INDEX_FILE", &index)
                .args(args)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {args:?}");
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        git(&["read-tree", &tip]);
        let entry = format!("100644,{blob},{path}");
        git(&["update-index", "--add", "--cacheinfo", &entry]);
        let tree = git(&["write-tree"]);
        let later = git_out(
            &self.repo,
            &["commit-tree", &tree, "-p", &tip, "-m", "More."],
        );
        git_out(
            &self.repo,
            &["update-ref", &format!("refs/heads/{branch}"), &later],
        );
        std::fs::remove_file(&index).unwrap();
        later
    }
}

/// The text of a `Conflict` refusal; anything else fails the test.
fn conflict<T: std::fmt::Debug>(result: dots_core::Result<T>) -> String {
    match result {
        Err(Error::Conflict(text)) => text,
        other => panic!("expected Conflict, got {other:?}"),
    }
}

/// Why an approval or a diff of `branch` is refused when the page saw another tip.
fn changed(branch: &str) -> String {
    format!("The branches changed since the check was shown: {branch}. Look at the check again.")
}

#[tokio::test]
async fn a_check_shows_its_branches_at_their_tips_and_the_approval_records_what_it_showed() {
    let w = world().await;
    let flow = w.ship_flow(DeployMode::Branch).await;
    let (wr, branch, tip) = w.at_the_check(&flow, false).await;
    let dev = newest(&wr, "dev").handoff.clone().unwrap();
    let run_id = dev.branches[0].run_id.clone();
    let view = w.workflows.check(&wr.id).await.unwrap();
    assert_eq!(
        view,
        CheckView {
            step_id: "check".into(),
            step_run_id: newest(&wr, "check").id.clone(),
            summary: "Fixed the icon.".into(),
            branches: vec![ShownBranch {
                branch: branch.clone(),
                run_id: run_id.clone(),
                commit: Some(tip.clone()),
            }],
        }
    );
    // What the app is given.
    assert_eq!(
        serde_json::to_value(&view).unwrap(),
        json!({
            "step_id": "check",
            "step_run_id": newest(&wr, "check").id,
            "summary": "Fixed the icon.",
            "branches": [{"branch": branch, "run_id": run_id, "commit": tip}],
        })
    );
    assert_eq!(
        view.tips(),
        vec![CheckTip {
            branch: branch.clone(),
            commit: Some(tip.clone()),
        }]
    );
    // Reading it changes nothing.
    let still = w.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(still.waiting_on, Some(WaitingOn::Check));
    assert_eq!(newest(&still, "check").decision, None);

    w.workflows
        .approve_check(&wr.id, Some("Ship it."), &view.step_run_id, &view.tips())
        .await
        .unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(
        newest(&wr, "check").decision,
        Some(json!({
            "decision": "approved",
            "note": "Ship it.",
            "branches": [{"branch": branch, "commit": tip}],
        }))
    );
    assert_eq!(
        conflict(w.workflows.check(&wr.id).await),
        format!("workflow run {} is not waiting for a check", wr.id)
    );
}

#[tokio::test]
async fn an_approval_of_tips_git_no_longer_has_is_refused_and_records_nothing() {
    let w = world().await;
    let flow = w.ship_flow(DeployMode::Branch).await;
    let (wr, branch, _) = w.at_the_check(&flow, false).await;
    let view = w.workflows.check(&wr.id).await.unwrap();
    let seen = view.tips();
    // The branch moves on after the page read the check.
    let later = w.commit_after(&branch);
    let approve = |tips: Vec<CheckTip>| {
        let w = &w;
        let id = wr.id.clone();
        let step_run = view.step_run_id.clone();
        async move {
            w.workflows
                .approve_check(&id, Some("Ship it."), &step_run, &tips)
                .await
        }
    };
    assert_eq!(conflict(approve(seen).await), changed(&branch));
    let fresh = w.workflows.check(&wr.id).await.unwrap().tips();
    assert_eq!(
        fresh,
        vec![CheckTip {
            branch: branch.clone(),
            commit: Some(later.clone()),
        }]
    );
    // A page that showed no branch, one the check does not show, or the same one twice.
    assert_eq!(conflict(approve(vec![]).await), changed(&branch));
    let mut more = fresh.clone();
    more.push(CheckTip {
        branch: "main".into(),
        commit: None,
    });
    assert_eq!(conflict(approve(more).await), changed("main"));
    let twice = vec![fresh[0].clone(), fresh[0].clone()];
    assert_eq!(conflict(approve(twice).await), changed(&branch));
    // Nothing was recorded.
    let still = w.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(still.status, WorkflowRunStatus::Waiting);
    assert_eq!(still.waiting_on, Some(WaitingOn::Check));
    assert_eq!(newest(&still, "check").status, StepRunStatus::Waiting);
    assert_eq!(newest(&still, "check").decision, None);
    assert!(still.steps.iter().all(|s| s.step_id != "ship"));

    // Read again, the approval records the tip the page now showed, and the deploy lists it.
    approve(fresh).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(
        newest(&wr, "check").decision.as_ref().unwrap()["branches"],
        json!([{"branch": branch, "commit": later}])
    );
    let listed = newest(&wr, "ship").deploy_result.clone().unwrap();
    assert_eq!(listed.branches[0].commit, later);
}

/// Why an approval or a send-back is refused when the check the page showed no longer waits.
const CHECK_CHANGED: &str = "The check changed since it was shown. Look at the check again.";

#[tokio::test]
async fn an_approval_of_a_check_that_no_longer_waits_is_refused_and_records_nothing() {
    let w = world().await;
    let flow = w
        .workflow("Look twice", vec![check("check"), check("again")])
        .await;
    let wr = w.start(&flow).await;
    let wr = w.until(&wr.id, at_check(1)).await;
    let first = w.workflows.check(&wr.id).await.unwrap();
    assert_eq!(first.step_run_id, newest(&wr, "check").id);
    w.workflows
        .approve_check(&wr.id, None, &first.step_run_id, &first.tips())
        .await
        .unwrap();
    let wr = w.until(&wr.id, at("again", 1)).await;
    let second = w.workflows.check(&wr.id).await.unwrap();
    // The second check shows what the first showed: only its step run tells them apart.
    assert_eq!(second.tips(), first.tips());
    assert_eq!(second.summary, first.summary);
    assert_eq!(second.step_run_id, newest(&wr, "again").id);
    assert_ne!(second.step_run_id, first.step_run_id);

    // A page still showing the first check is refused, before its tips are compared.
    assert_eq!(
        conflict(
            w.workflows
                .approve_check(&wr.id, Some("Twice."), &first.step_run_id, &first.tips())
                .await
        ),
        CHECK_CHANGED
    );
    let other = vec![CheckTip {
        branch: "main".into(),
        commit: None,
    }];
    assert_eq!(
        conflict(
            w.workflows
                .approve_check(&wr.id, None, &first.step_run_id, &other)
                .await
        ),
        CHECK_CHANGED
    );
    // Nothing was recorded.
    let still = w.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(still.status, WorkflowRunStatus::Waiting);
    assert_eq!(still.waiting_on, Some(WaitingOn::Check));
    assert_eq!(
        attempts(&still),
        [
            ("check", 1, StepRunStatus::Succeeded),
            ("again", 1, StepRunStatus::Waiting)
        ]
    );
    assert_eq!(newest(&still, "again").decision, None);

    // The second check's own view is approved.
    w.workflows
        .approve_check(&wr.id, None, &second.step_run_id, &second.tips())
        .await
        .unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    assert_eq!(
        newest(&wr, "again").decision,
        Some(json!({"decision": "approved", "note": null, "branches": []}))
    );
}

#[tokio::test]
async fn a_send_back_from_a_check_that_no_longer_waits_is_refused_and_records_nothing() {
    let w = world().await;
    w.engine.set(
        &w.dev.spec.name,
        vec![
            Act::Write("icon.txt", "Made the icon visible."),
            Act::Finish("Kept it as it was."),
        ],
    );
    let flow = w
        .workflow(
            "Fix",
            vec![
                agent("dev", "Fix it", &w.dev, "Fix the bug."),
                check("check"),
            ],
        )
        .await;
    let wr = w.start(&flow).await;
    w.until(&wr.id, at_check(1)).await;
    let first = w.workflows.check(&wr.id).await.unwrap();
    w.workflows
        .send_back_check(&wr.id, "Use the light set.", &first.step_run_id)
        .await
        .unwrap();
    let wr = w.until(&wr.id, at_check(2)).await;
    let second = w.workflows.check(&wr.id).await.unwrap();
    assert_eq!(second.step_run_id, newest(&wr, "check").id);
    assert_ne!(second.step_run_id, first.step_run_id);

    // A page still showing the first check is refused.
    assert_eq!(
        conflict(
            w.workflows
                .send_back_check(&wr.id, "Again.", &first.step_run_id)
                .await
        ),
        CHECK_CHANGED
    );
    // Nothing was recorded.
    let still = w.store.get_workflow_run(&wr.id).await.unwrap();
    assert_eq!(still.status, WorkflowRunStatus::Waiting);
    assert_eq!(still.waiting_on, Some(WaitingOn::Check));
    assert_eq!(still.send_backs, 1);
    assert_eq!(attempts(&still), attempts(&wr));
    assert_eq!(newest(&still, "check").decision, None);
    assert_eq!(w.engine.prompts_of("dev").len(), 2);
}

#[tokio::test]
async fn a_branch_s_diff_is_its_work_from_the_default_branch_and_other_refs_are_refused() {
    let w = world().await;
    let flow = w.ship_flow(DeployMode::Branch).await;
    let (wr, branch, tip) = w.at_the_check(&flow, false).await;
    // The default branch moves on meanwhile: its new work is not the branch's.
    std::fs::write(w.repo.join("later.txt"), "On main.\n").unwrap();
    git_out(&w.repo, &["add", "later.txt"]);
    git_out(&w.repo, &["commit", "-q", "-m", "Later on main."]);
    let main = git_out(&w.repo, &["rev-parse", "refs/heads/main"]);

    let diff = w
        .workflows
        .branch_diff(&wr.id, &branch, &tip)
        .await
        .unwrap();
    let files: Vec<_> = diff
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.status))
        .collect();
    assert_eq!(files, [("fix.txt", FileStatus::Added)]);
    assert!(
        diff.files[0].patch.contains("+Fixed the icon."),
        "{}",
        diff.files[0].patch
    );
    assert!(!diff.truncated);
    assert!(diff.exists);

    // Only a branch the run shows: not the default branch, a ref, or an option.
    for name in ["main", "HEAD", "--output=x.txt"] {
        match w.workflows.branch_diff(&wr.id, name, &tip).await {
            Err(Error::Invalid(m)) => {
                assert_eq!(m, format!("workflow run {} shows no branch {name}", wr.id))
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
    }
    // And only at the commit it shows: not a ref, another commit, or an option.
    let before = format!("{tip}~1");
    for commit in [
        "HEAD",
        "main",
        main.as_str(),
        before.as_str(),
        "--output=x.txt",
    ] {
        assert_eq!(
            conflict(w.workflows.branch_diff(&wr.id, &branch, commit).await),
            changed(&branch)
        );
    }
    assert!(!w.repo.join("x.txt").exists());

    // Past the cap, the later files are listed without their patch.
    let big = "x".repeat(dots_core::workspace::MAX_DIFF_BYTES) + "\n";
    let moved = w.commit_file(&branch, "big.txt", &big);
    let diff = w
        .workflows
        .branch_diff(&wr.id, &branch, &moved)
        .await
        .unwrap();
    let files: Vec<_> = diff
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.status, f.patch.is_empty()))
        .collect();
    assert_eq!(
        files,
        [
            ("big.txt", FileStatus::Added, true),
            ("fix.txt", FileStatus::Added, false),
        ]
    );
    assert!(diff.truncated);
    // The check now shows the new tip, so the old one is refused.
    assert_eq!(
        conflict(w.workflows.branch_diff(&wr.id, &branch, &tip).await),
        changed(&branch)
    );

    // Once approved and listed by the deploy, the branch is diffed at the commit recorded.
    w.approve(&wr.id, None).await.unwrap();
    let wr = w
        .until(&wr.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    let diff = w
        .workflows
        .branch_diff(&wr.id, &branch, &moved)
        .await
        .unwrap();
    assert_eq!(diff.files.len(), 2);
    assert_eq!(
        conflict(w.workflows.branch_diff(&wr.id, &branch, &tip).await),
        changed(&branch)
    );
    // With no default branch to diff from, it says so.
    git_out(&w.repo, &["checkout", "-q", "--detach"]);
    assert_eq!(
        conflict(w.workflows.branch_diff(&wr.id, &branch, &moved).await),
        NO_DEFAULT_BRANCH
    );
}

impl World {
    /// Starts a run of `workflow` as `start` does, a moment after the one before (runs started
    /// in one millisecond would sort by their random ids), from the chat run `origin` and as a
    /// test run when asked.
    async fn start_from(&self, workflow: &str, origin: Option<&str>, test: bool) -> WorkflowRun {
        tokio::time::sleep(Duration::from_millis(5)).await;
        self.workflows
            .start(StartRequest {
                workflow_id: workflow.into(),
                project_id: self.project.clone(),
                task: "The tray icon is invisible in dark mode.".into(),
                trigger: if origin.is_some() {
                    WorkflowTrigger::Chat
                } else {
                    WorkflowTrigger::Manual
                },
                origin_run_id: origin.map(str::to_string),
                test,
            })
            .await
            .unwrap()
    }

    /// The ids of the workflow runs `filter` lists, in its order.
    async fn listed(&self, filter: WorkflowRunFilter) -> Vec<String> {
        let runs = self.workflows.list(&filter).await.unwrap();
        runs.into_iter().map(|wr| wr.id).collect()
    }
}

#[tokio::test]
async fn workflow_runs_are_listed_by_workflow_and_status_and_found_by_the_chat_that_started_them() {
    let w = world().await;
    let fix = w.workflow("Fix", vec![check("check")]).await;
    let other = w.workflow("Other", vec![check("check")]).await;
    let chat = w
        .store
        .create_run(&NewRun::new(&w.helper.id, TriggerKind::Chat))
        .await
        .unwrap()
        .id;
    let stopped = w.start_from(&fix, None, false).await;
    w.until(&stopped.id, waits_on(WaitingOn::Check)).await;
    w.workflows.stop(&stopped.id).await.unwrap();
    let carded = w.start_from(&fix, Some(&chat), false).await;
    let elsewhere = w.start_from(&other, None, true).await;
    for wr in [&carded, &elsewhere] {
        w.until(&wr.id, waits_on(WaitingOn::Check)).await;
    }

    let fix_runs = WorkflowRunFilter {
        team_id: Some(w.team.clone()),
        workflow_id: Some(fix.clone()),
        ..Default::default()
    };
    assert_eq!(
        w.listed(fix_runs.clone()).await,
        [carded.id.as_str(), stopped.id.as_str()]
    );
    assert_eq!(
        w.listed(WorkflowRunFilter {
            statuses: vec![WorkflowRunStatus::Waiting],
            ..Default::default()
        })
        .await,
        [elsewhere.id.as_str(), carded.id.as_str()]
    );
    assert_eq!(
        w.listed(WorkflowRunFilter {
            statuses: vec![WorkflowRunStatus::Cancelled],
            ..fix_runs
        })
        .await,
        [stopped.id.as_str()]
    );
    assert_eq!(
        w.listed(WorkflowRunFilter {
            limit: Some(1),
            ..Default::default()
        })
        .await,
        [elsewhere.id.as_str()]
    );

    // The chat card: the runs its chat started, whole.
    let cards = w
        .workflows
        .for_origins(std::slice::from_ref(&chat))
        .await
        .unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].id, carded.id);
    assert_eq!(cards[0].origin_run_id.as_deref(), Some(chat.as_str()));
    assert_eq!(cards[0].waiting_on, Some(WaitingOn::Check));
    assert!(w.workflows.for_origins(&[]).await.unwrap().is_empty());

    // Each workflow says what runs it has in flight and which started last.
    let fix_now = w.store.get_workflow(&fix).await.unwrap();
    assert_eq!(fix_now.runs_in_flight, 1);
    let last = fix_now.last_run.unwrap();
    assert_eq!(
        (last.id.as_str(), last.status, last.test),
        (carded.id.as_str(), WorkflowRunStatus::Waiting, false)
    );
    let other_now = w.store.get_workflow(&other).await.unwrap();
    assert_eq!(other_now.runs_in_flight, 1);
    assert!(other_now.last_run.unwrap().test);
    w.workflows.stop(&carded.id).await.unwrap();
    assert_eq!(w.store.get_workflow(&fix).await.unwrap().runs_in_flight, 0);
}

#[tokio::test]
async fn deleting_a_team_announces_each_workflow_it_removes() {
    let w = world().await;
    w.engine.set(&w.dev.spec.name, vec![Act::Hold]);
    let fix = w
        .workflow("Fix", vec![agent("dev", "Fix it", &w.dev, "Fix.")])
        .await;
    let ship = w.workflow("Ship", vec![check("check")]).await;
    let wr = w.start(&fix).await;
    w.until(&wr.id, |wr| wr.current_step.as_deref() == Some("dev"))
        .await;
    let mut events = w.bus.subscribe();
    w.workflows.delete_team(&w.team).await.unwrap();
    let mut seen = Vec::new();
    while let Ok(e) = events.try_recv() {
        match e {
            // A pass may announce the run before the stop; the stop announces it cancelled.
            RuntimeEvent::WorkflowRunUpdated { run }
                if run.status == WorkflowRunStatus::Cancelled =>
            {
                seen.push("run cancelled".to_string())
            }
            RuntimeEvent::WorkflowUpdated {
                team_id,
                workflow_id,
            } => {
                assert_eq!(team_id, w.team);
                let name = if workflow_id == fix { "Fix" } else { "?" };
                let name = if workflow_id == ship { "Ship" } else { name };
                seen.push(format!("workflow {name}"));
            }
            RuntimeEvent::TeamUpdated { team_id } => {
                assert_eq!(team_id, w.team);
                seen.push("team".into());
            }
            _ => {}
        }
    }
    assert_eq!(
        seen,
        ["run cancelled", "workflow Fix", "workflow Ship", "team"]
    );
}

/// The run page's links: every dot run of the workflow run's steps, with the step run each works
/// for, oldest first. A delegate step's lead, its member and the run that continues the lead
/// with the member's result are all the delegate step's; the continuation is in the lead's
/// thread, the member's run is its own. They are the runs the budget counts.
#[tokio::test]
async fn a_workflow_run_lists_its_steps_dot_runs_with_their_threads() {
    let w = world().await;
    w.engine.set(
        &w.helper.spec.name,
        vec![
            Act::Write("plan.txt", "The plan."),
            Act::Start(
                vec![("dev", "Make the menu dark.")],
                Box::new(Act::Finish("Started one.")),
            ),
            Act::Finish("It is in."),
        ],
    );
    w.engine.set(
        &w.dev.spec.name,
        vec![Act::Write("part.txt", "Did my part.")],
    );
    let flow = w
        .workflow(
            "Fix",
            vec![
                agent("plan", "Plan it", &w.helper, "Plan."),
                delegating("split", "Split it", &w.helper, &[&w.dev], 1, 1),
            ],
        )
        .await;
    let started = w.start(&flow).await;
    let wr = w
        .until(&started.id, |wr| wr.status == WorkflowRunStatus::Succeeded)
        .await;
    let (plan, split) = (&wr.steps[0].id, &wr.steps[1].id);
    let got = w.workflows.dot_runs(&wr.id).await.unwrap();
    let rows: Vec<_> = got
        .iter()
        .map(|r| {
            let step = if &r.step_run_id == plan {
                "plan"
            } else {
                "split"
            };
            (step, r.dot_name.as_str(), r.status)
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("plan", "helper", RunStatus::Succeeded),
            ("split", "helper", RunStatus::Succeeded),
            ("split", "dev", RunStatus::Succeeded),
            ("split", "helper", RunStatus::Succeeded),
        ]
    );
    assert_eq!(got[1].step_run_id, *split);
    assert_eq!(got[3].root_run_id, got[1].run_id);
    assert_eq!(got[2].root_run_id, got[2].run_id);
    assert_eq!(got[2].dot_id, w.dev.id);
    let mut own = w.store.runs_of_step_run(plan).await.unwrap();
    own.extend(w.store.runs_of_step_run(split).await.unwrap());
    assert_eq!(
        got.iter().map(|r| &r.run_id).collect::<Vec<_>>(),
        own.iter().map(|r| &r.id).collect::<Vec<_>>()
    );
    assert_eq!(wr.spent_runs as usize, got.len());
    assert!(matches!(
        w.workflows.dot_runs("nope").await,
        Err(Error::NotFound(m)) if m == "workflow run nope"
    ));
}
