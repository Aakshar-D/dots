use dots_core::workflow::{Budget, Definition, Step, StepKind, Trigger};
use dots_core::Error;
use serde_json::json;

fn agent(id: &str, member: &str) -> Step {
    Step {
        id: id.into(),
        name: format!("Step {id}"),
        kind: StepKind::Agent {
            member: member.into(),
            job: "Do it.".into(),
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
    assert_eq!(d.steps[1].kind, StepKind::Check {});
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
    assert_eq!(
        d.step("c").map(|(i, s)| (i, s.name.as_str())),
        Some((1, "Your check"))
    );
    assert!(d.step("z").is_none());
}
