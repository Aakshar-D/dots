use dots_core::schedule::{Rule, Schedule};
use dots_core::workflow::{
    BranchOn, BranchPath, Budget, Definition, Delegate, DeployMode, MergeMethod, Step, StepKind,
    Trigger,
};
use dots_core::Error;
use serde_json::json;

fn agent(id: &str, member: &str) -> Step {
    Step {
        id: id.into(),
        name: format!("Step {id}"),
        kind: StepKind::Agent {
            member: member.into(),
            job: "Do it.".into(),
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

fn def(steps: Vec<Step>) -> Definition {
    Definition {
        trigger: Trigger::Manual,
        steps,
        budget: Budget::default(),
    }
}

fn members() -> Vec<String> {
    vec!["helper".into(), "dev".into()]
}

fn refusal(d: &Definition) -> String {
    match d.validate(&members()) {
        Err(Error::Invalid(m)) => m,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[test]
fn the_json_shape_round_trips() {
    let value = json!({
        "trigger": {"kind": "chat", "starters": ["helper"]},
        "steps": [
            {"id": "a", "name": "Helper", "kind": "agent", "member": "helper", "job": "Write the task."},
            {"id": "c", "name": "Your check", "kind": "check"}
        ],
        "budget": {"max_runs": 5}
    });
    let d: Definition = serde_json::from_value(value).unwrap();
    assert_eq!(
        d.trigger,
        Trigger::Chat {
            starters: vec!["helper".into()]
        }
    );
    assert_eq!(d.steps[1].kind, StepKind::Check { send_back_to: None });
    assert_eq!(d.budget.max_runs, 5);
    assert_eq!(d.budget.max_tokens, 1_500_000);
    assert_eq!(d.budget.max_minutes, 120);
    assert_eq!(d.budget.max_send_backs, 3);
    let again: Definition = serde_json::from_value(serde_json::to_value(&d).unwrap()).unwrap();
    assert_eq!(again, d);
}

#[test]
fn a_missing_budget_takes_the_defaults() {
    let d: Definition = serde_json::from_value(json!({
        "trigger": {"kind": "manual"},
        "steps": [{"id": "c", "name": "Check", "kind": "check"}]
    }))
    .unwrap();
    assert_eq!(d.budget, Budget::default());
}

#[test]
fn a_good_definition_passes() {
    def(vec![agent("a", "helper"), agent("b", "dev"), check("c")])
        .validate(&members())
        .unwrap();
}

#[test]
fn the_rules_of_section_5_8() {
    assert_eq!(refusal(&def(vec![])), "a workflow needs at least one step");
    assert_eq!(
        refusal(&def(vec![agent("a", "stranger")])),
        "step \"Step a\": stranger is not a member of the team"
    );
    assert_eq!(
        refusal(&def(vec![agent("a", "dev"), agent("a", "dev")])),
        "step \"Step a\": another step has the id \"a\""
    );
    assert_eq!(
        refusal(&def(vec![agent("Bad Id", "dev")])),
        "step \"Step Bad Id\": its id must be 1 to 40 of a-z, 0-9, '-' and '_'"
    );
    let mut blank = agent("a", "dev");
    blank.name = "  ".into();
    assert_eq!(
        refusal(&def(vec![blank])),
        "step \"a\": a name must be 1 to 80 characters"
    );
    let mut no_job = agent("a", "dev");
    no_job.kind = StepKind::Agent {
        member: "dev".into(),
        job: " ".into(),
        can_send_back_to: vec![],
        outcomes: vec![],
        delegate: None,
    };
    assert_eq!(
        refusal(&def(vec![no_job])),
        "step \"Step a\": the job must be 1 to 8,000 characters"
    );
    let mut zero = def(vec![check("c")]);
    zero.budget.max_minutes = 0;
    assert_eq!(refusal(&zero), "every budget limit must be at least 1");
    let mut many = def((0..51).map(|i| check(&format!("c{i}"))).collect());
    assert_eq!(refusal(&many), "a workflow has at most 50 steps");
    many.steps.truncate(50);
    many.validate(&members()).unwrap();
}

#[test]
fn a_chat_trigger_names_members_who_start_it() {
    let mut d = def(vec![check("c")]);
    d.trigger = Trigger::Chat { starters: vec![] };
    assert_eq!(
        refusal(&d),
        "a chat trigger needs at least one member who starts it"
    );
    d.trigger = Trigger::Chat {
        starters: vec!["ghost".into()],
    };
    assert_eq!(
        refusal(&d),
        "the trigger names ghost, who is not a member of the team"
    );
    d.trigger = Trigger::Chat {
        starters: vec!["helper".into()],
    };
    d.validate(&members()).unwrap();
    assert!(d.started_from_chat_by("helper"));
    assert!(!d.started_from_chat_by("dev"));
}

#[test]
fn steps_are_found_by_id() {
    let d = def(vec![agent("a", "helper"), check("c")]);
    assert_eq!(d.step("c").map(|s| s.name.as_str()), Some("Your check"));
    assert_eq!(d.position("c"), Some(1));
    assert!(d.step("z").is_none());
}

fn with_outcomes(mut step: Step, labels: &[&str]) -> Step {
    if let StepKind::Agent { outcomes, .. } = &mut step.kind {
        *outcomes = labels.iter().map(|l| l.to_string()).collect();
    }
    step
}

fn sending_back_to(mut step: Step, ids: &[&str]) -> Step {
    if let StepKind::Agent {
        can_send_back_to, ..
    } = &mut step.kind
    {
        *can_send_back_to = ids.iter().map(|i| i.to_string()).collect();
    }
    step
}

fn delegating(mut step: Step, may_start: &[&str], at_once: u32, total: u32) -> Step {
    if let StepKind::Agent { delegate, .. } = &mut step.kind {
        *delegate = Some(Delegate {
            may_start: may_start.iter().map(|m| m.to_string()).collect(),
            at_once,
            total,
        });
    }
    step
}

fn check_to(id: &str, to: &str) -> Step {
    Step {
        id: id.into(),
        name: "Your check".into(),
        kind: StepKind::Check {
            send_back_to: Some(to.into()),
        },
    }
}

fn deploy(id: &str, mode: DeployMode) -> Step {
    Step {
        id: id.into(),
        name: format!("Deploy {id}"),
        kind: StepKind::Deploy { mode },
    }
}

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

fn daily() -> Schedule {
    serde_json::from_value(json!({"rule": {"kind": "daily", "time": "09:00"}})).unwrap()
}

#[test]
fn plan_a_definitions_still_read() {
    let d: Definition = serde_json::from_str(
        r#"{"trigger":{"kind":"manual"},"steps":[
            {"id":"a","name":"Fix it","kind":"agent","member":"dev","job":"Fix the bug."},
            {"id":"c","name":"Your check","kind":"check"}
        ],"budget":{"max_runs":10,"max_tokens":1500000,"max_minutes":120,"max_send_backs":3}}"#,
    )
    .unwrap();
    assert_eq!(
        d.steps[0].kind,
        StepKind::Agent {
            member: "dev".into(),
            job: "Fix the bug.".into(),
            can_send_back_to: vec![],
            outcomes: vec![],
            delegate: None,
        }
    );
    assert_eq!(d.steps[1].kind, StepKind::Check { send_back_to: None });
    d.validate(&members()).unwrap();
}

#[test]
fn the_new_shapes_round_trip() {
    let value = json!({
        "trigger": {"kind": "webhook", "task": "Fix what the report says."},
        "steps": [
            {"id": "tm", "name": "Plan", "kind": "agent", "member": "helper", "job": "Split it.",
             "delegate": {"may_start": ["dev"]}},
            {"id": "dev", "name": "Fix", "kind": "agent", "member": "dev", "job": "Fix it.",
             "can_send_back_to": ["tm"], "outcomes": ["fixed", "needs_info"]},
            {"id": "br", "name": "Which", "kind": "branch", "on": "outcome", "paths": [
                {"label": "fixed", "steps": [
                    {"id": "c", "name": "Your check", "kind": "check", "send_back_to": "dev"},
                    {"id": "ship", "name": "Ship", "kind": "deploy", "mode": "pr_merge"}
                ]}
            ], "otherwise": [
                {"id": "ask", "name": "Ask", "kind": "agent", "member": "helper", "job": "Ask."}
            ]},
            {"id": "c2", "name": "Release check", "kind": "check"},
            {"id": "rel", "name": "Release", "kind": "deploy", "mode": "member", "member": "dev",
             "job": "Tag a release."}
        ]
    });
    let d: Definition = serde_json::from_value(value).unwrap();
    assert_eq!(
        d.trigger,
        Trigger::Webhook {
            task: Some("Fix what the report says.".into()),
            project: None
        }
    );
    let StepKind::Agent { delegate, .. } = &d.steps[0].kind else {
        panic!("not an agent step: {:?}", d.steps[0]);
    };
    assert_eq!(
        delegate,
        &Some(Delegate {
            may_start: vec!["dev".into()],
            at_once: 3,
            total: 6
        })
    );
    assert_eq!(
        d.step("ship").unwrap().kind,
        StepKind::Deploy {
            mode: DeployMode::PrMerge {
                merge_method: MergeMethod::Squash
            }
        }
    );
    assert_eq!(
        d.step("rel").unwrap().kind,
        StepKind::Deploy {
            mode: DeployMode::Member {
                member: "dev".into(),
                job: "Tag a release.".into()
            }
        }
    );
    d.validate(&members()).unwrap();
    let out = serde_json::to_value(&d).unwrap();
    assert_eq!(out["steps"][2]["paths"][0]["steps"][1]["mode"], "pr_merge");
    assert_eq!(
        out["steps"][2]["paths"][0]["steps"][1]["merge_method"],
        "squash"
    );
    let again: Definition = serde_json::from_value(out).unwrap();
    assert_eq!(again, d);
}

#[test]
fn a_schedule_trigger_round_trips_and_is_checked() {
    let mut d = def(vec![check("c")]);
    d.trigger = Trigger::Schedule {
        schedule: daily(),
        task: "Look for new issues.".into(),
        project: "Dots".into(),
    };
    d.validate(&members()).unwrap();
    let value = serde_json::to_value(&d).unwrap();
    assert_eq!(value["trigger"]["kind"], "schedule");
    assert_eq!(value["trigger"]["schedule"]["rule"]["kind"], "daily");
    assert_eq!(value["trigger"]["project"], "Dots");
    let again: Definition = serde_json::from_value(value).unwrap();
    assert_eq!(again, d);

    let with = |schedule: Schedule, task: &str, project: &str| {
        let mut d = def(vec![check("c")]);
        d.trigger = Trigger::Schedule {
            schedule,
            task: task.into(),
            project: project.into(),
        };
        d
    };
    let mut bad = daily();
    bad.rule = Rule::Weekly {
        days: vec![],
        time: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
    };
    assert_eq!(
        refusal(&with(bad, "Look.", "Dots")),
        "the trigger's schedule: weekly: pick at least one day"
    );
    assert_eq!(
        refusal(&with(daily(), " ", "Dots")),
        "the trigger's task must be 1 to 8,000 characters"
    );
    assert_eq!(
        refusal(&with(daily(), &"x".repeat(8_001), "Dots")),
        "the trigger's task must be 1 to 8,000 characters"
    );
    with(daily(), &"x".repeat(8_000), "Dots")
        .validate(&members())
        .unwrap();
    assert_eq!(
        refusal(&with(daily(), "Look.", "  ")),
        "the trigger needs a project"
    );
}

#[test]
fn a_webhook_trigger_s_fallback_task_is_at_most_8000_characters() {
    let mut d = def(vec![check("c")]);
    d.trigger = Trigger::Webhook {
        task: Some("x".repeat(8_001)),
        project: None,
    };
    assert_eq!(
        refusal(&d),
        "the trigger's task must be 1 to 8,000 characters"
    );
    d.trigger = Trigger::Webhook {
        task: None,
        project: Some("Dots".into()),
    };
    d.validate(&members()).unwrap();
}

#[test]
fn a_delegate_step_may_start_only_members() {
    assert_eq!(
        refusal(&def(vec![delegating(
            agent("a", "helper"),
            &["ghost"],
            3,
            6
        )])),
        "step \"Step a\": it may start ghost, who is not a member of the team"
    );
    assert_eq!(
        refusal(&def(vec![delegating(agent("a", "helper"), &[], 3, 6)])),
        "step \"Step a\": a delegate step needs at least one member it may start"
    );
    def(vec![delegating(agent("a", "helper"), &["dev"], 3, 6)])
        .validate(&members())
        .unwrap();
}

#[test]
fn a_delegate_step_s_limits() {
    let limits = |at_once, total| {
        def(vec![delegating(
            agent("a", "helper"),
            &["dev"],
            at_once,
            total,
        )])
    };
    assert_eq!(
        refusal(&limits(0, 0)),
        "step \"Step a\": the members it may start in all must be at least 1"
    );
    assert_eq!(
        refusal(&limits(0, 6)),
        "step \"Step a\": the members it may start at once must be 1 to 6"
    );
    assert_eq!(
        refusal(&limits(7, 6)),
        "step \"Step a\": the members it may start at once must be 1 to 6"
    );
    limits(6, 6).validate(&members()).unwrap();
    limits(1, 1).validate(&members()).unwrap();
}

#[test]
fn outcomes_are_1_to_40_characters_and_listed_once() {
    assert_eq!(
        refusal(&def(vec![with_outcomes(
            agent("a", "dev"),
            &["fixed", " "]
        )])),
        "step \"Step a\": an outcome must be 1 to 40 characters"
    );
    let long = "x".repeat(41);
    assert_eq!(
        refusal(&def(vec![with_outcomes(agent("a", "dev"), &[&long])])),
        "step \"Step a\": an outcome must be 1 to 40 characters"
    );
    assert_eq!(
        refusal(&def(vec![with_outcomes(
            agent("a", "dev"),
            &["fixed", " fixed"]
        )])),
        "step \"Step a\": the outcome \"fixed\" is listed twice"
    );
    def(vec![with_outcomes(
        agent("a", "dev"),
        &["fixed", "Fixed", &"x".repeat(40)],
    )])
    .validate(&members())
    .unwrap();
}

#[test]
fn an_agent_sends_back_only_to_an_agent_step_before_it() {
    let refused = |to: &str| {
        format!(
            "step \"Step b\": work can be sent back only to an agent step before it, not to \
             \"{to}\""
        )
    };
    // A later step, a check, an unknown id, itself.
    assert_eq!(
        refusal(&def(vec![
            sending_back_to(agent("b", "dev"), &["c"]),
            agent("c", "dev")
        ])),
        refused("c")
    );
    assert_eq!(
        refusal(&def(vec![
            check("k"),
            sending_back_to(agent("b", "dev"), &["k"])
        ])),
        refused("k")
    );
    assert_eq!(
        refusal(&def(vec![sending_back_to(agent("b", "dev"), &["zz"])])),
        refused("zz")
    );
    assert_eq!(
        refusal(&def(vec![sending_back_to(agent("b", "dev"), &["b"])])),
        refused("b")
    );
    // A step inside an earlier branch's path is on another path.
    assert_eq!(
        refusal(&def(vec![
            branch(
                "br",
                BranchOn::Project,
                vec![("Dots", vec![agent("in", "dev")])],
                vec![]
            ),
            sending_back_to(agent("b", "dev"), &["in"]),
        ])),
        refused("in")
    );
    def(vec![
        agent("a", "helper"),
        sending_back_to(agent("b", "dev"), &["a"]),
    ])
    .validate(&members())
    .unwrap();
    // From inside a path, back to a step before the branch.
    def(vec![
        agent("a", "helper"),
        branch(
            "br",
            BranchOn::Project,
            vec![("Dots", vec![sending_back_to(agent("b", "dev"), &["a"])])],
            vec![],
        ),
    ])
    .validate(&members())
    .unwrap();
}

#[test]
fn a_check_sends_back_only_to_an_agent_step_before_it() {
    let refused = |to: &str| {
        format!(
            "step \"Your check\": work can be sent back only to an agent step before it, not \
             to \"{to}\""
        )
    };
    assert_eq!(
        refusal(&def(vec![check_to("k", "a"), agent("a", "dev")])),
        refused("a")
    );
    assert_eq!(
        refusal(&def(vec![check("k1"), check_to("k2", "k1")])),
        refused("k1")
    );
    def(vec![agent("a", "dev"), check("k1"), check_to("k2", "a")])
        .validate(&members())
        .unwrap();
}

#[test]
fn a_deploy_needs_a_check_before_it_on_every_path() {
    let refused = "step \"Deploy d\": a deploy needs a check before it on every path";
    assert_eq!(
        refusal(&def(vec![agent("a", "dev"), deploy("d", DeployMode::Push)])),
        refused
    );
    // A check after it does not count.
    assert_eq!(
        refusal(&def(vec![deploy("d", DeployMode::Pr), check("k")])),
        refused
    );
    // A branch whose `otherwise` has no check leaves a path without one.
    assert_eq!(
        refusal(&def(vec![
            branch(
                "br",
                BranchOn::Project,
                vec![("Dots", vec![check("k")])],
                vec![]
            ),
            deploy("d", DeployMode::Push),
        ])),
        refused
    );
    // A nested branch with a path without a check.
    assert_eq!(
        refusal(&def(vec![
            branch(
                "br",
                BranchOn::Project,
                vec![(
                    "Dots",
                    vec![branch(
                        "inner",
                        BranchOn::Project,
                        vec![("Dots", vec![check("k1")])],
                        vec![agent("a", "dev")],
                    )]
                )],
                vec![check("k2")],
            ),
            deploy("d", DeployMode::Push),
        ])),
        refused
    );
    // A check after it in its own path does not count, even when the branch's other lists
    // hold one.
    assert_eq!(
        refusal(&def(vec![
            agent("a", "dev"),
            branch(
                "br",
                BranchOn::Project,
                vec![("Dots", vec![deploy("d", DeployMode::Push), check("k1")])],
                vec![check("k2")],
            ),
        ])),
        refused
    );
    def(vec![check("k"), deploy("d", DeployMode::Push)])
        .validate(&members())
        .unwrap();
    // Every path and `otherwise` holds a check, one of them in a nested branch.
    def(vec![
        branch(
            "br",
            BranchOn::Project,
            vec![(
                "Dots",
                vec![branch(
                    "inner",
                    BranchOn::Project,
                    vec![("Dots", vec![agent("a", "dev"), check("k1")])],
                    vec![check("k2")],
                )],
            )],
            vec![check("k3")],
        ),
        deploy("d", DeployMode::Push),
    ])
    .validate(&members())
    .unwrap();
    // Inside a path, after a check of that path or before the branch.
    def(vec![branch(
        "br",
        BranchOn::Project,
        vec![("Dots", vec![check("k"), deploy("d", DeployMode::Branch)])],
        vec![],
    )])
    .validate(&members())
    .unwrap();
    def(vec![
        check("k"),
        branch(
            "br",
            BranchOn::Project,
            vec![("Dots", vec![deploy("d", DeployMode::Branch)])],
            vec![],
        ),
    ])
    .validate(&members())
    .unwrap();
}

#[test]
fn a_deploy_member_is_a_member_with_a_job() {
    let member = |member: &str, job: &str| {
        def(vec![
            check("k"),
            deploy(
                "d",
                DeployMode::Member {
                    member: member.into(),
                    job: job.into(),
                },
            ),
        ])
    };
    assert_eq!(
        refusal(&member("ghost", "Tag it.")),
        "step \"Deploy d\": ghost is not a member of the team"
    );
    assert_eq!(
        refusal(&member("dev", " ")),
        "step \"Deploy d\": the job must be 1 to 8,000 characters"
    );
    assert_eq!(
        refusal(&member("dev", &"x".repeat(8_001))),
        "step \"Deploy d\": the job must be 1 to 8,000 characters"
    );
    member("dev", "Tag it.").validate(&members()).unwrap();
}

#[test]
fn a_branch_on_outcome_uses_the_labels_of_the_nearest_agent_step() {
    let on_outcome = |labels: &[&str]| {
        branch(
            "br",
            BranchOn::Outcome,
            labels.iter().map(|l| (*l, vec![])).collect(),
            vec![],
        )
    };
    assert_eq!(
        refusal(&def(vec![check("c"), on_outcome(&["fixed"])])),
        "step \"Branch br\": a branch on outcome needs an agent step before it"
    );
    assert_eq!(
        refusal(&def(vec![
            with_outcomes(agent("a", "dev"), &["fixed"]),
            on_outcome(&["fixed", "needs_info"]),
        ])),
        "step \"Branch br\": \"needs_info\" is not an outcome of the step \"Step a\""
    );
    // The nearest agent step's labels, not an earlier one's.
    assert_eq!(
        refusal(&def(vec![
            with_outcomes(agent("a", "dev"), &["fixed"]),
            agent("b", "helper"),
            on_outcome(&["fixed"]),
        ])),
        "step \"Branch br\": \"fixed\" is not an outcome of the step \"Step b\""
    );
    def(vec![
        with_outcomes(agent("a", "dev"), &["fixed", "needs_info"]),
        check("c"),
        on_outcome(&["fixed", "needs_info"]),
    ])
    .validate(&members())
    .unwrap();
    // A branch on project needs no outcomes.
    def(vec![branch(
        "br",
        BranchOn::Project,
        vec![("Dots", vec![check("k")])],
        vec![],
    )])
    .validate(&members())
    .unwrap();
}

#[test]
fn a_branch_s_labels_are_given_once() {
    let labels = |labels: &[&str]| {
        def(vec![branch(
            "br",
            BranchOn::Project,
            labels.iter().map(|l| (*l, vec![])).collect(),
            vec![],
        )])
    };
    assert_eq!(
        refusal(&labels(&["Dots", " "])),
        "step \"Branch br\": every path needs a label"
    );
    assert_eq!(
        refusal(&labels(&["Dots", "dots "])),
        "step \"Branch br\": two paths have the label \"dots\""
    );
    assert_eq!(
        refusal(&labels(&[])),
        "step \"Branch br\": a branch needs at least one path"
    );
    labels(&["Dots", "Other app"]).validate(&members()).unwrap();
}

#[test]
fn ids_are_unique_and_names_checked_across_the_tree() {
    assert_eq!(
        refusal(&def(vec![
            agent("a", "dev"),
            branch(
                "br",
                BranchOn::Project,
                vec![("Dots", vec![agent("a", "dev")])],
                vec![]
            ),
        ])),
        "step \"Step a\": another step has the id \"a\""
    );
    assert_eq!(
        refusal(&def(vec![branch(
            "br",
            BranchOn::Project,
            vec![("Dots", vec![])],
            vec![agent("a", "ghost")]
        )])),
        "step \"Step a\": ghost is not a member of the team"
    );
}

#[test]
fn the_step_limit_counts_the_steps_inside_branches() {
    let nested = |n: usize| {
        def(vec![branch(
            "br",
            BranchOn::Project,
            vec![("Dots", (0..n).map(|i| check(&format!("c{i}"))).collect())],
            vec![],
        )])
    };
    assert_eq!(refusal(&nested(50)), "a workflow has at most 50 steps");
    nested(49).validate(&members()).unwrap();
}

/// `a`, then a branch on its outcome: `fixed` runs a check, a nested branch on project
/// (`Dots`: a deploy; otherwise `x`) and `after`; `needs_info` runs `h`; otherwise `o`. Then
/// `z`.
fn tree() -> Definition {
    def(vec![
        with_outcomes(agent("a", "helper"), &["fixed", "needs_info"]),
        branch(
            "br",
            BranchOn::Outcome,
            vec![
                (
                    "fixed",
                    vec![
                        check("c1"),
                        branch(
                            "nb",
                            BranchOn::Project,
                            vec![("Dots", vec![deploy("d1", DeployMode::Push)])],
                            vec![agent("x", "dev")],
                        ),
                        agent("after", "dev"),
                    ],
                ),
                ("needs_info", vec![agent("h", "helper")]),
            ],
            vec![agent("o", "dev")],
        ),
        check("z"),
    ])
}

fn ids(steps: Vec<&Step>) -> Vec<&str> {
    steps.into_iter().map(|s| s.id.as_str()).collect()
}

#[test]
fn all_lists_the_tree_in_run_order() {
    let d = tree();
    d.validate(&members()).unwrap();
    assert_eq!(
        ids(d.all()),
        ["a", "br", "c1", "nb", "d1", "x", "after", "h", "o", "z"]
    );
    assert_eq!(d.position("a"), Some(0));
    assert_eq!(d.position("d1"), Some(4));
    assert_eq!(d.position("z"), Some(9));
    assert_eq!(d.position("nope"), None);
    assert_eq!(d.step("d1").map(|s| s.name.as_str()), Some("Deploy d1"));
    assert_eq!(d.first().map(|s| s.id.as_str()), Some("a"));
    assert!(def(vec![]).first().is_none());
}

#[test]
fn next_after_climbs_out_of_ended_paths() {
    let d = tree();
    let next = |id: &str| d.next_after(id).map(|s| s.id.as_str());
    assert_eq!(next("a"), Some("br"));
    // A branch's next is the step after the whole branch.
    assert_eq!(next("br"), Some("z"));
    assert_eq!(next("c1"), Some("nb"));
    // The end of a nested path goes on in the path around it.
    assert_eq!(next("d1"), Some("after"));
    assert_eq!(next("x"), Some("after"));
    // The end of a path goes on after its branch.
    assert_eq!(next("after"), Some("z"));
    assert_eq!(next("h"), Some("z"));
    assert_eq!(next("o"), Some("z"));
    assert_eq!(next("z"), None);
    assert_eq!(next("nope"), None);
}

#[test]
fn before_lists_the_steps_on_the_way_to_a_step() {
    let d = tree();
    assert!(d.before("a").is_empty());
    assert_eq!(ids(d.before("br")), ["a"]);
    assert_eq!(ids(d.before("c1")), ["a", "br"]);
    assert_eq!(ids(d.before("d1")), ["a", "br", "c1", "nb"]);
    assert_eq!(ids(d.before("x")), ["a", "br", "c1", "nb"]);
    assert_eq!(ids(d.before("after")), ["a", "br", "c1", "nb"]);
    assert_eq!(ids(d.before("h")), ["a", "br"]);
    assert_eq!(ids(d.before("o")), ["a", "br"]);
    // Steps inside the branch's paths are not on the way to the step after it.
    assert_eq!(ids(d.before("z")), ["a", "br"]);
    assert!(d.before("nope").is_empty());
}
