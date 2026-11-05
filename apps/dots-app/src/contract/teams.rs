//! The samples of teams and workflows (teams spec sections 3 to 11): teams with their members
//! and projects, workflows with every step kind, deploy mode and trigger kind, workflow runs in
//! each wait with their step runs, hand-offs, decisions and deploy results, a check, the
//! builder's problems, and their events.

use dots_core::deploy::{DeployResult, DeployedBranch};
use dots_core::events::RuntimeEvent;
use dots_core::model::*;
use dots_core::schedule::Schedule;
use dots_core::workflow::{
    BranchOn, BranchPath, Budget, Definition, Delegate, DeployMode, MergeMethod, Step, StepKind,
    Trigger, WorkflowProblem,
};
use dots_core::workflows::{CheckView, ShownBranch};
use serde_json::{json, Value};

use super::{to, TS};

const TEAM: &str = "01J0TEAM";
const DEV: &str = "01J0DOT";
const REVIEWER: &str = "01J0DOT2";
const PROJECT: &str = "01J0PRJ";
const REPO: &str = "C:/Users/u/src/dots";
const TASK: &str = "The tray icon is invisible on a light taskbar.\nIt shows on a dark one.";

fn project_spec() -> ProjectSpec {
    ProjectSpec {
        name: "Dots".into(),
        repo_path: REPO.into(),
        default_branch: Some("main".into()),
        remote: "origin".into(),
    }
}

fn project() -> Project {
    Project {
        id: PROJECT.into(),
        team_id: TEAM.into(),
        created_at: TS.into(),
        updated_at: TS.into(),
        spec: project_spec(),
    }
}

fn team_spec() -> TeamSpec {
    TeamSpec {
        name: "crew".into(),
        description: "Fixes and ships Dots.".into(),
        members: vec![
            MemberSpec {
                dot_id: DEV.into(),
                role: "dev agent".into(),
            },
            MemberSpec {
                dot_id: REVIEWER.into(),
                role: String::new(),
            },
        ],
        org_id: None,
        workspace_id: "default".into(),
    }
}

/// A Personal team with two members and two projects (one finds its default branch), and an
/// org's team with none.
fn teams() -> Vec<Team> {
    vec![
        Team {
            id: TEAM.into(),
            name: "crew".into(),
            description: "Fixes and ships Dots.".into(),
            members: vec![
                TeamMember {
                    dot_id: DEV.into(),
                    dot_name: "nightly-tests".into(),
                    role: "dev agent".into(),
                },
                TeamMember {
                    dot_id: REVIEWER.into(),
                    dot_name: "reviewer".into(),
                    role: String::new(),
                },
            ],
            projects: vec![
                project(),
                Project {
                    id: "01J0PRJ2".into(),
                    spec: ProjectSpec {
                        name: "Site".into(),
                        repo_path: "C:/Users/u/src/site".into(),
                        default_branch: None,
                        remote: "upstream".into(),
                    },
                    ..project()
                },
            ],
            org_id: None,
            workspace_id: "default".into(),
            created_at: TS.into(),
            updated_at: TS.into(),
        },
        Team {
            id: "01J0TEAM2".into(),
            name: "platform".into(),
            description: String::new(),
            members: vec![],
            projects: vec![],
            org_id: Some("acme-eng".into()),
            workspace_id: "platform".into(),
            created_at: TS.into(),
            updated_at: TS.into(),
        },
    ]
}

fn agent(id: &str, name: &str, member: &str, job: &str) -> Step {
    Step {
        id: id.into(),
        name: name.into(),
        kind: StepKind::Agent {
            member: member.into(),
            job: job.into(),
            can_send_back_to: vec![],
            outcomes: vec![],
            delegate: None,
        },
    }
}

fn check(id: &str, send_back_to: Option<&str>) -> Step {
    Step {
        id: id.into(),
        name: "Review".into(),
        kind: StepKind::Check {
            send_back_to: send_back_to.map(Into::into),
        },
    }
}

fn deploy(id: &str, mode: DeployMode) -> Step {
    Step {
        id: id.into(),
        name: "Ship".into(),
        kind: StepKind::Deploy { mode },
    }
}

/// A lead that may start members and reports an outcome, then a branch on it: "fixed" goes
/// to a check and a pull request that merges itself, anything else to a designer who may send
/// the work back to the lead.
fn chat_definition() -> Definition {
    Definition {
        trigger: Trigger::Chat {
            starters: vec![DEV.into()],
        },
        steps: vec![
            Step {
                id: "build".into(),
                name: "Build".into(),
                kind: StepKind::Agent {
                    member: DEV.into(),
                    job: "Fix the bug and say whether it is fixed.".into(),
                    can_send_back_to: vec![],
                    outcomes: vec!["fixed".into(), "needs_design".into()],
                    delegate: Some(Delegate {
                        may_start: vec![REVIEWER.into()],
                        at_once: 2,
                        total: 4,
                    }),
                },
            },
            Step {
                id: "route".into(),
                name: "Fixed?".into(),
                kind: StepKind::Branch {
                    on: BranchOn::Outcome,
                    paths: vec![BranchPath {
                        label: "fixed".into(),
                        steps: vec![
                            check("review", Some("build")),
                            deploy(
                                "ship",
                                DeployMode::PrMerge {
                                    merge_method: MergeMethod::Squash,
                                },
                            ),
                        ],
                    }],
                    otherwise: vec![Step {
                        id: "design".into(),
                        name: "Design".into(),
                        kind: StepKind::Agent {
                            member: REVIEWER.into(),
                            job: "Propose a design.".into(),
                            can_send_back_to: vec!["build".into()],
                            outcomes: vec![],
                            delegate: None,
                        },
                    }],
                },
            },
        ],
        budget: Budget::default(),
    }
}

fn workflow(id: &str, name: &str, definition: Definition) -> Workflow {
    Workflow {
        id: id.into(),
        team_id: TEAM.into(),
        created_at: TS.into(),
        updated_at: TS.into(),
        problem: None,
        has_webhook_token: false,
        runs_in_flight: 0,
        last_run: None,
        spec: WorkflowSpec {
            name: name.into(),
            definition,
            enabled: true,
        },
    }
}

/// One workflow per trigger kind; between them every step kind and deploy mode, a branch on
/// outcome with an otherwise and one on project without, and a delegate.
fn workflows() -> Vec<Workflow> {
    let work = || agent("work", "Work", DEV, "Do the task.");
    let schedule: Schedule = serde_json::from_value(
        json!({"rule": {"kind": "daily", "time": "09:00"}, "catch_up": false, "until": null}),
    )
    .expect("sample schedule");
    vec![
        Workflow {
            runs_in_flight: 1,
            last_run: Some(LastRun {
                id: "01J0WFR".into(),
                status: WorkflowRunStatus::Waiting,
                started_at: TS.into(),
                test: false,
            }),
            ..workflow("01J0WFL", "Fix and ship", chat_definition())
        },
        Workflow {
            problem: Some("step \"Work\": 01J0GONE is not a member of the team".into()),
            last_run: Some(LastRun {
                id: "01J0WFR4".into(),
                status: WorkflowRunStatus::Succeeded,
                started_at: TS.into(),
                test: true,
            }),
            spec: WorkflowSpec {
                name: "Push it".into(),
                definition: Definition {
                    trigger: Trigger::Manual,
                    steps: vec![
                        agent("work", "Work", "01J0GONE", "Do the task."),
                        check("check", None),
                        deploy("push", DeployMode::Push),
                    ],
                    budget: Budget {
                        max_runs: 4,
                        max_tokens: 200_000,
                        max_minutes: 30,
                        max_send_backs: 1,
                    },
                },
                enabled: false,
            },
            ..workflow("01J0WFL2", "", chat_definition())
        },
        workflow(
            "01J0WFL3",
            "Nightly",
            Definition {
                trigger: Trigger::Schedule {
                    schedule,
                    task: "Update the dependencies.".into(),
                    project: "Dots".into(),
                },
                steps: vec![
                    work(),
                    check("check", None),
                    Step {
                        id: "by-project".into(),
                        name: "Which project".into(),
                        kind: StepKind::Branch {
                            on: BranchOn::Project,
                            paths: vec![
                                BranchPath {
                                    label: "Dots".into(),
                                    steps: vec![deploy("pr", DeployMode::Pr)],
                                },
                                BranchPath {
                                    label: "Site".into(),
                                    steps: vec![deploy("keep", DeployMode::Branch)],
                                },
                            ],
                            otherwise: vec![],
                        },
                    },
                ],
                budget: Budget::default(),
            },
        ),
        Workflow {
            has_webhook_token: true,
            ..workflow(
                "01J0WFL4",
                "On issue",
                Definition {
                    trigger: Trigger::Webhook {
                        task: Some("Look at the newest issue.".into()),
                        project: None,
                    },
                    steps: vec![
                        work(),
                        check("check", None),
                        deploy(
                            "release",
                            DeployMode::Member {
                                member: REVIEWER.into(),
                                job: "Release what was approved.".into(),
                            },
                        ),
                    ],
                    budget: Budget::default(),
                },
            )
        },
    ]
}

fn branch_ref(run_id: &str, branch: &str) -> BranchRef {
    BranchRef {
        run_id: run_id.into(),
        branch: branch.into(),
        worktree_path: format!("C:/Users/u/AppData/Local/dots/worktrees/{run_id}"),
        base_commit: Some("1111111111111111111111111111111111111111".into()),
    }
}

fn step_run(id: &str, step_id: &str, status: StepRunStatus) -> StepRun {
    StepRun {
        id: id.into(),
        workflow_run_id: "01J0WFR".into(),
        step_id: step_id.into(),
        attempt: 1,
        status,
        handoff: None,
        decision: None,
        error: None,
        outcome: None,
        notice: None,
        send_back: None,
        deploy_result: None,
        started_at: TS.into(),
        ended_at: status.is_terminal().then(|| TS.into()),
    }
}

/// The lead's hand-off: its own branch and the member branch it did not merge, with what each
/// member it started handed back.
fn lead_handoff() -> Handoff {
    Handoff {
        summary: "Made the icon dark on a light taskbar.".into(),
        branches: vec![
            branch_ref("01J0RUN5", "dots/nightly-tests/01J0RUN5"),
            branch_ref("01J0RUN6", "dots/reviewer/01J0RUN6"),
        ],
        outcome: Some("fixed".into()),
        members: vec![
            MemberHandoff {
                dot_id: REVIEWER.into(),
                task: "Check the icon on Windows 10.".into(),
                status: RunStatus::Succeeded,
                summary: "It shows on both.".into(),
                branch: Some("dots/reviewer/01J0RUN6".into()),
            },
            MemberHandoff {
                dot_id: REVIEWER.into(),
                task: "Check the high-contrast theme.".into(),
                status: RunStatus::Failed,
                summary: "No high-contrast theme on this PC.".into(),
                branch: None,
            },
        ],
    }
}

fn workflow_run(id: &str) -> WorkflowRun {
    WorkflowRun {
        id: id.into(),
        workflow_id: Some("01J0WFL".into()),
        team_id: TEAM.into(),
        definition: chat_definition(),
        project_id: Some(PROJECT.into()),
        project: project_spec(),
        task: TASK.into(),
        trigger: WorkflowTrigger::Chat,
        origin_run_id: Some("01J0RUN".into()),
        test: false,
        status: WorkflowRunStatus::Running,
        waiting_on: None,
        current_step: Some("build".into()),
        budget: Budget::default(),
        send_backs: 0,
        send_back: None,
        spent_runs: 3,
        spent_tokens: 120_000,
        spent_cache_read: 900_000,
        active_ms: 300_000,
        error: None,
        started_at: TS.into(),
        ended_at: None,
        steps: vec![],
    }
}

/// Waiting on a check, after a lead with members and a branch on its outcome; one that went
/// back once and was approved and shipped; one holding a send-back at the cap; a test run whose
/// deploy failed part way; one started from the team page and held by the budget, whose workflow
/// and project were deleted since.
fn workflow_runs() -> Vec<WorkflowRun> {
    let lead = StepRun {
        handoff: Some(lead_handoff()),
        outcome: Some("fixed".into()),
        ..step_run("01J0STP1", "build", StepRunStatus::Succeeded)
    };
    let route = StepRun {
        decision: Some(json!({"path": "fixed"})),
        ..step_run("01J0STP2", "route", StepRunStatus::Succeeded)
    };
    let tips = json!([
        {"branch": "dots/nightly-tests/01J0RUN5", "commit": "2222222222222222222222222222222222222222"},
        {"branch": "dots/reviewer/01J0RUN6", "commit": null},
    ]);
    let pushed = DeployedBranch {
        branch: "dots/nightly-tests/01J0RUN5".into(),
        commit: "2222222222222222222222222222222222222222".into(),
        pushed: true,
        pr_url: Some("https://github.com/Aakshar-D/dots/pull/40".into()),
        auto_merge: true,
    };
    vec![
        WorkflowRun {
            status: WorkflowRunStatus::Waiting,
            waiting_on: Some(WaitingOn::Check),
            current_step: Some("review".into()),
            steps: vec![
                lead.clone(),
                route.clone(),
                step_run("01J0STP3", "review", StepRunStatus::Waiting),
            ],
            ..workflow_run("01J0WFR")
        },
        WorkflowRun {
            status: WorkflowRunStatus::Succeeded,
            current_step: None,
            send_backs: 1,
            send_back: Some(SendBack {
                from: "review".into(),
                to: "build".into(),
                note: "Keep the old icon for dark mode.".into(),
                by: SentBy::Person,
                held: false,
            }),
            ended_at: Some(TS.into()),
            steps: vec![
                lead.clone(),
                route.clone(),
                StepRun {
                    decision: Some(json!({
                        "decision": "sent_back",
                        "note": "Keep the old icon for dark mode.",
                    })),
                    ..step_run("01J0STP3", "review", StepRunStatus::Succeeded)
                },
                StepRun {
                    id: "01J0STP9".into(),
                    attempt: 2,
                    notice: Some("Its earlier chat could not go on, so it started fresh.".into()),
                    ..lead.clone()
                },
                StepRun {
                    id: "01J0STPA".into(),
                    ..route.clone()
                },
                StepRun {
                    attempt: 2,
                    handoff: Some(lead_handoff()),
                    decision: Some(json!({"decision": "approved", "note": null, "branches": tips})),
                    ..step_run("01J0STP4", "review", StepRunStatus::Succeeded)
                },
                StepRun {
                    deploy_result: Some(DeployResult {
                        branches: vec![pushed.clone()],
                        ready: false,
                        error: None,
                    }),
                    ..step_run("01J0STP5", "ship", StepRunStatus::Succeeded)
                },
            ],
            ..workflow_run("01J0WFR2")
        },
        WorkflowRun {
            status: WorkflowRunStatus::Waiting,
            waiting_on: Some(WaitingOn::SendBacks),
            current_step: Some("design".into()),
            send_backs: 3,
            send_back: Some(SendBack {
                from: "design".into(),
                to: "build".into(),
                note: "The icon needs a new shape first.".into(),
                by: SentBy::Agent,
                held: true,
            }),
            steps: vec![StepRun {
                outcome: None,
                send_back: Some(
                    json!({"to": "build", "note": "The icon needs a new shape first."}),
                ),
                ..step_run("01J0STP6", "design", StepRunStatus::Waiting)
            }],
            ..workflow_run("01J0WFR3")
        },
        WorkflowRun {
            test: true,
            trigger: WorkflowTrigger::Manual,
            origin_run_id: None,
            status: WorkflowRunStatus::Waiting,
            waiting_on: Some(WaitingOn::StepFailed),
            current_step: Some("ship".into()),
            error: Some("Ship: gh: could not create the pull request".into()),
            steps: vec![StepRun {
                error: Some("gh: could not create the pull request".into()),
                deploy_result: Some(DeployResult {
                    branches: vec![
                        DeployedBranch {
                            pr_url: None,
                            auto_merge: false,
                            ..pushed.clone()
                        },
                        DeployedBranch {
                            branch: "dots/reviewer/01J0RUN6".into(),
                            commit: "3333333333333333333333333333333333333333".into(),
                            pushed: false,
                            pr_url: None,
                            auto_merge: false,
                        },
                    ],
                    ready: false,
                    error: Some("gh: could not create the pull request".into()),
                }),
                ..step_run("01J0STP7", "ship", StepRunStatus::Failed)
            }],
            ..workflow_run("01J0WFR4")
        },
        // Nightly's run on Site, which took the "Site" path to keep its branch.
        WorkflowRun {
            workflow_id: None,
            definition: workflows()[2].spec.definition.clone(),
            project_id: None,
            project: teams()[0].projects[1].spec.clone(),
            trigger: WorkflowTrigger::Manual,
            origin_run_id: None,
            status: WorkflowRunStatus::Waiting,
            waiting_on: Some(WaitingOn::Budget),
            current_step: Some("keep".into()),
            budget: Budget {
                max_runs: 3,
                ..Budget::default()
            },
            steps: vec![StepRun {
                deploy_result: Some(DeployResult {
                    branches: vec![DeployedBranch {
                        pushed: false,
                        pr_url: None,
                        auto_merge: false,
                        ..pushed
                    }],
                    ready: true,
                    error: None,
                }),
                ..step_run("01J0STP8", "keep", StepRunStatus::Succeeded)
            }],
            ..workflow_run("01J0WFR5")
        },
    ]
}

/// What the waiting check of the first run shows: the lead's branch at its tip, and a member
/// branch that is gone.
fn check_view() -> CheckView {
    CheckView {
        step_id: "review".into(),
        summary: "Made the icon dark on a light taskbar.".into(),
        branches: vec![
            ShownBranch {
                branch: "dots/nightly-tests/01J0RUN5".into(),
                run_id: "01J0RUN5".into(),
                commit: Some("2222222222222222222222222222222222222222".into()),
            },
            ShownBranch {
                branch: "dots/reviewer/01J0RUN6".into(),
                run_id: "01J0RUN6".into(),
                commit: None,
            },
        ],
    }
}

/// The dot runs of the run waiting on its check, oldest first: the lead's run, the run of each
/// member it started (one handed back a branch, one failed without), and the run that continued
/// the lead with their results, in the lead's thread.
fn step_dot_runs() -> Vec<StepDotRun> {
    let lead = (DEV, "nightly-tests");
    let member = (REVIEWER, "reviewer");
    let run = |(dot, name): (&str, &str), run_id: &str, root: &str, status| StepDotRun {
        step_run_id: "01J0STP1".into(),
        run_id: run_id.into(),
        root_run_id: root.into(),
        dot_id: dot.into(),
        dot_name: name.into(),
        status,
        started_at: Some(TS.into()),
    };
    vec![
        run(lead, "01J0RUN5", "01J0RUN5", RunStatus::Succeeded),
        run(member, "01J0RUN6", "01J0RUN6", RunStatus::Succeeded),
        run(member, "01J0RUN8", "01J0RUN8", RunStatus::Failed),
        run(lead, "01J0RUN7", "01J0RUN5", RunStatus::Succeeded),
    ]
}

/// Adds the samples of teams, workflows and workflow runs, and their events.
pub(super) fn teams_samples(samples: &mut Value) {
    samples["Team"] = to(teams());
    samples["TeamSpec"] = to(team_spec());
    samples["Project"] = to(project());
    samples["ProjectSpec"] = to([
        project_spec(),
        ProjectSpec {
            default_branch: None,
            ..project_spec()
        },
    ]);
    let workflows = workflows();
    samples["WorkflowSpec"] = to(workflows.iter().map(|w| &w.spec).collect::<Vec<_>>());
    samples["Workflow"] = to(&workflows);
    // The builder's live check: a step's problem, then the workflow's.
    samples["WorkflowProblem"] = to([
        WorkflowProblem {
            step_id: Some("work".into()),
            message: "step \"Work\": 01J0GONE is not a member of the team".into(),
        },
        WorkflowProblem {
            step_id: None,
            message: "a workflow name must be 1 to 64 characters".into(),
        },
    ]);
    samples["WorkflowRunFilter"] = to([
        WorkflowRunFilter {
            team_id: Some(TEAM.into()),
            workflow_id: Some("01J0WFL".into()),
            statuses: vec![WorkflowRunStatus::Running, WorkflowRunStatus::Waiting],
            limit: Some(20),
        },
        WorkflowRunFilter::default(),
    ]);
    let runs = workflow_runs();
    samples["WorkflowRun"] = to(&runs);
    samples["CheckView"] = to(check_view());
    samples["CheckTip"] = to(check_view().tips());
    samples["StepDotRun"] = to(step_dot_runs());
    samples["Enums"]["WorkflowTrigger"] = to(WorkflowTrigger::ALL);
    samples["Enums"]["WorkflowRunStatus"] = to(WorkflowRunStatus::ALL);
    samples["Enums"]["StepRunStatus"] = to(StepRunStatus::ALL);
    samples["RuntimeEvent"]
        .as_array_mut()
        .expect("the RuntimeEvent samples")
        .extend([
            to(RuntimeEvent::TeamUpdated {
                team_id: TEAM.into(),
            }),
            to(RuntimeEvent::WorkflowUpdated {
                team_id: TEAM.into(),
                workflow_id: "01J0WFL".into(),
            }),
            to(RuntimeEvent::WorkflowRunUpdated {
                run: runs[0].clone(),
            }),
        ]);
}
