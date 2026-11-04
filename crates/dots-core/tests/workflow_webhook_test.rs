//! A workflow's webhook (teams spec section 5.1): `POST /workflows/{id}/trigger` with the
//! workflow's token starts a run on the body's task and project, each falling back to the
//! webhook trigger's own, through Dots' own server. The workflow's one step is the person's
//! check, so a started run waits there and no engine runs. A role that may not run dots is
//! refused in `runtime_roles_test` (it needs an account's member list).

mod common;

use std::path::PathBuf;

use common::{init_repo, spec};
use dots_core::model::{
    Dot, MemberSpec, ProjectSpec, TeamSpec, Workflow, WorkflowRun, WorkflowSpec, WorkflowTrigger,
};
use dots_core::workflow::{Budget, Definition, Step, StepKind, Trigger};
use dots_core::{Config, Runtime};
use serde_json::{json, Value};
use tempfile::TempDir;

/// The task the trigger falls back to in these tests.
const TRIGGER_TASK: &str = "Triage the crash reports that came in.";

struct Rt {
    _dir: TempDir,
    rt: Runtime,
    team: String,
    dev: Dot,
    other: Dot,
    http: reqwest::Client,
}

impl Rt {
    /// A runtime on Dots' own server (a free port), with a team of `dev` and `other` and two
    /// projects, "Dots" and "Site".
    async fn start() -> Rt {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::new(dir.path().join("data"));
        cfg.port = 0;
        // Never the real CLI: no run here reaches an engine.
        cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
        let rt = Runtime::start(cfg).await.unwrap();
        let dev = rt
            .store()
            .create_dot(&spec("dev"), None, "default")
            .await
            .unwrap();
        let other = rt
            .store()
            .create_dot(&spec("other"), None, "default")
            .await
            .unwrap();
        let team = rt
            .teams()
            .create(&TeamSpec {
                org_id: None,
                workspace_id: "default".into(),
                name: "Dots management".into(),
                description: String::new(),
                members: vec![member(&dev), member(&other)],
            })
            .await
            .unwrap()
            .id;
        for name in ["Dots", "Site"] {
            let repo = dir.path().join(name.to_lowercase());
            init_repo(&repo);
            rt.teams()
                .add_project(
                    &team,
                    &ProjectSpec {
                        name: name.into(),
                        repo_path: repo.to_string_lossy().to_string(),
                        default_branch: None,
                        remote: "origin".into(),
                    },
                )
                .await
                .unwrap();
        }
        Rt {
            _dir: dir,
            rt,
            team,
            dev,
            other,
            http: reqwest::Client::new(),
        }
    }

    /// The id of the team's project named `name`.
    async fn project(&self, name: &str) -> String {
        let team = self.rt.teams().get(&self.team).await.unwrap();
        let project = team.projects.iter().find(|p| p.spec.name == name).unwrap();
        project.id.clone()
    }

    /// A workflow "Fix" with `trigger` and the person's check as its one step.
    async fn workflow(&self, trigger: Trigger) -> Workflow {
        self.rt
            .teams()
            .create_workflow(&self.team, &flow(trigger, check()))
            .await
            .unwrap()
    }

    /// A webhook workflow whose trigger falls back to `task` and `project`, and its token.
    async fn hooked(&self, task: Option<&str>, project: Option<&str>) -> (Workflow, String) {
        let workflow = self.workflow(webhook(task, project)).await;
        let token = self
            .rt
            .teams()
            .regenerate_workflow_token(&workflow.id)
            .await
            .unwrap();
        (workflow, token)
    }

    fn url(&self, workflow_id: &str) -> String {
        format!(
            "http://127.0.0.1:{}/workflows/{workflow_id}/trigger",
            self.rt.port()
        )
    }

    /// Posts `body` (raw, sent as JSON) to the workflow's webhook with `token` as the bearer;
    /// the status and the JSON answer.
    async fn post(
        &self,
        workflow_id: &str,
        token: Option<&str>,
        body: Option<&str>,
    ) -> (u16, Value) {
        let mut req = self.http.post(self.url(workflow_id));
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        if let Some(body) = body {
            req = req
                .header("content-type", "application/json")
                .body(body.to_string());
        }
        let resp = req.send().await.unwrap();
        let status = resp.status().as_u16();
        (status, resp.json().await.unwrap())
    }

    /// The workflow run a 202 answer named.
    async fn started(&self, answer: (u16, Value)) -> WorkflowRun {
        let (status, body) = answer;
        assert_eq!(status, 202, "{body}");
        let id = body["workflow_run_id"].as_str().unwrap();
        self.rt.store().get_workflow_run(id).await.unwrap()
    }

    async fn runs(&self) -> usize {
        self.rt
            .store()
            .list_workflow_runs(Some(&self.team), 100)
            .await
            .unwrap()
            .len()
    }
}

fn member(dot: &Dot) -> MemberSpec {
    MemberSpec {
        dot_id: dot.id.clone(),
        role: "dev agent".into(),
    }
}

fn check() -> Step {
    Step {
        id: "check".into(),
        name: "Your check".into(),
        kind: StepKind::Check { send_back_to: None },
    }
}

fn webhook(task: Option<&str>, project: Option<&str>) -> Trigger {
    Trigger::Webhook {
        task: task.map(str::to_string),
        project: project.map(str::to_string),
    }
}

fn flow(trigger: Trigger, step: Step) -> WorkflowSpec {
    WorkflowSpec {
        name: "Fix".into(),
        definition: Definition {
            trigger,
            steps: vec![step],
            budget: Budget::default(),
        },
        enabled: true,
    }
}

fn error(answer: &(u16, Value)) -> &str {
    answer.1["error"].as_str().unwrap()
}

#[tokio::test]
async fn a_call_starts_a_run_on_the_body_s_task_and_project_found_without_case() {
    let t = Rt::start().await;
    let (workflow, token) = t.hooked(Some(TRIGGER_TASK), Some("Dots")).await;
    let body = json!({"task": "  Fix the login crash.  ", "project": " site "}).to_string();
    let wr = t
        .started(t.post(&workflow.id, Some(&token), Some(&body)).await)
        .await;
    assert_eq!(wr.workflow_id.as_deref(), Some(workflow.id.as_str()));
    assert_eq!(wr.task, "Fix the login crash.");
    assert_eq!(wr.project_id, Some(t.project("Site").await));
    assert_eq!(wr.project.name, "Site");
    assert_eq!(wr.trigger, WorkflowTrigger::Webhook);
    assert_eq!(wr.origin_run_id, None);
    assert!(!wr.test);
    t.rt.shutdown();
}

#[tokio::test]
async fn the_body_falls_back_to_the_trigger_s_task_and_project() {
    let t = Rt::start().await;
    let (workflow, token) = t.hooked(Some(TRIGGER_TASK), Some("dots")).await;
    let dots = t.project("Dots").await;
    // No body.
    let wr = t
        .started(t.post(&workflow.id, Some(&token), None).await)
        .await;
    assert_eq!(
        (wr.task.as_str(), wr.project_id.clone()),
        (TRIGGER_TASK, Some(dots.clone()))
    );
    // A blank body, an empty object, blank values and nulls.
    for body in [
        "  ",
        "{}",
        r#"{"task": "  ", "project": ""}"#,
        r#"{"task": null, "project": null}"#,
    ] {
        let wr = t
            .started(t.post(&workflow.id, Some(&token), Some(body)).await)
            .await;
        assert_eq!(
            (wr.task.as_str(), wr.project_id.clone()),
            (TRIGGER_TASK, Some(dots.clone())),
            "{body}"
        );
    }
    // Only the task, with another sender's own keys, which are ignored.
    let body = json!({"task": "Fix the build.", "action": "opened", "number": 7}).to_string();
    let wr = t
        .started(t.post(&workflow.id, Some(&token), Some(&body)).await)
        .await;
    assert_eq!(
        (wr.task.as_str(), wr.project_id),
        ("Fix the build.", Some(dots))
    );
    // Only the project.
    let body = json!({"project": "SITE"}).to_string();
    let wr = t
        .started(t.post(&workflow.id, Some(&token), Some(&body)).await)
        .await;
    assert_eq!(
        (wr.task.as_str(), wr.project_id),
        (TRIGGER_TASK, Some(t.project("Site").await))
    );
    assert_eq!(t.runs().await, 7);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_missing_task_or_project_or_an_unknown_project_is_400() {
    let t = Rt::start().await;
    let (workflow, token) = t.hooked(None, None).await;
    let answer = t.post(&workflow.id, Some(&token), None).await;
    assert_eq!(answer.0, 400);
    assert_eq!(
        error(&answer),
        "no task: give \"task\" in the body, or a task in the workflow's webhook trigger"
    );
    let body = json!({"task": "Fix it."}).to_string();
    let answer = t.post(&workflow.id, Some(&token), Some(&body)).await;
    assert_eq!(answer.0, 400);
    assert_eq!(
        error(&answer),
        "no project: give \"project\" in the body, or a project in the workflow's webhook trigger"
    );
    let body = json!({"task": "Fix it.", "project": " Docs "}).to_string();
    let answer = t.post(&workflow.id, Some(&token), Some(&body)).await;
    assert_eq!(answer.0, 400);
    assert_eq!(error(&answer), "the team has no project named \"Docs\"");
    // `Workflows::start`'s refusal of a task too long.
    let body = json!({"task": "x".repeat(8_001), "project": "Dots"}).to_string();
    let answer = t.post(&workflow.id, Some(&token), Some(&body)).await;
    assert_eq!(answer.0, 400);
    assert_eq!(error(&answer), "the task must be 1 to 8,000 characters");
    // A trigger project the team does not have.
    let (workflow, token) = {
        let mut spec = flow(webhook(Some(TRIGGER_TASK), Some("Gone")), check());
        spec.name = "Gone".into();
        let w = t.rt.teams().create_workflow(&t.team, &spec).await.unwrap();
        let token = t.rt.teams().regenerate_workflow_token(&w.id).await.unwrap();
        (w, token)
    };
    let answer = t.post(&workflow.id, Some(&token), None).await;
    assert_eq!(answer.0, 400);
    assert_eq!(error(&answer), "the team has no project named \"Gone\"");
    assert_eq!(t.runs().await, 0);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_body_that_is_not_json_of_that_shape_is_400() {
    let t = Rt::start().await;
    let (workflow, token) = t.hooked(Some(TRIGGER_TASK), Some("Dots")).await;
    let shape =
        "body must be a JSON object whose \"task\" and \"project\", when given, are strings";
    let answer = t.post(&workflow.id, Some(&token), Some("task=fix")).await;
    assert_eq!(answer.0, 400);
    assert!(
        error(&answer).starts_with("body must be JSON: "),
        "{}",
        error(&answer)
    );
    for body in [
        r#"["Fix it.", "Dots"]"#,
        r#""Fix it.""#,
        "7",
        r#"{"task": 7}"#,
        r#"{"project": ["Dots"]}"#,
    ] {
        let answer = t.post(&workflow.id, Some(&token), Some(body)).await;
        assert_eq!(answer.0, 400, "{body}");
        assert_eq!(error(&answer), shape, "{body}");
    }
    assert_eq!(t.runs().await, 0);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_missing_or_wrong_token_or_a_workflow_without_one_is_401() {
    let t = Rt::start().await;
    let workflow = t.workflow(webhook(Some(TRIGGER_TASK), Some("Dots"))).await;
    assert!(!workflow.has_webhook_token);
    for token in [None, Some(""), Some("anything")] {
        let answer = t.post(&workflow.id, token, None).await;
        assert_eq!(
            answer,
            (401, json!({"error": "invalid token"})),
            "{token:?}"
        );
    }
    let token =
        t.rt.teams()
            .regenerate_workflow_token(&workflow.id)
            .await
            .unwrap();
    assert_eq!(t.post(&workflow.id, None, None).await.0, 401);
    assert_eq!(t.post(&workflow.id, Some("wrong"), None).await.0, 401);
    // Its hash is not the token either.
    let hash = dots_core::util::token_hash(&token);
    assert_eq!(t.post(&workflow.id, Some(&hash), None).await.0, 401);
    // A token in the URL is refused, as a dot's: it would end up in proxy logs.
    let resp = t
        .http
        .post(format!("{}?token={token}", t.url(&workflow.id)))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 401);
    // Another workflow's token.
    let (_, other_token) = {
        let mut spec = flow(webhook(Some(TRIGGER_TASK), Some("Dots")), check());
        spec.name = "Other".into();
        let w = t.rt.teams().create_workflow(&t.team, &spec).await.unwrap();
        let token = t.rt.teams().regenerate_workflow_token(&w.id).await.unwrap();
        (w, token)
    };
    assert_eq!(t.post(&workflow.id, Some(&other_token), None).await.0, 401);
    assert_eq!(t.runs().await, 0);
    t.rt.shutdown();
}

#[tokio::test]
async fn an_unknown_workflow_is_404() {
    let t = Rt::start().await;
    let (workflow, token) = t.hooked(Some(TRIGGER_TASK), Some("Dots")).await;
    let answer = t.post("no-such-workflow", Some(&token), None).await;
    assert_eq!(answer, (404, json!({"error": "unknown workflow"})));
    t.rt.teams().delete_workflow(&workflow.id).await.unwrap();
    let answer = t.post(&workflow.id, Some(&token), None).await;
    assert_eq!(answer, (404, json!({"error": "unknown workflow"})));
    t.rt.shutdown();
}

#[tokio::test]
async fn a_workflow_without_a_webhook_trigger_turned_off_or_with_a_problem_is_409() {
    let t = Rt::start().await;
    let teams = t.rt.teams();
    // Another trigger: its token is kept, but it does not start from a webhook.
    let (workflow, token) = t.hooked(Some(TRIGGER_TASK), Some("Dots")).await;
    let manual = teams
        .update_workflow(&workflow.id, &flow(Trigger::Manual, check()))
        .await
        .unwrap();
    assert!(manual.has_webhook_token);
    let answer = t.post(&workflow.id, Some(&token), None).await;
    assert_eq!(
        answer,
        (
            409,
            json!({"error": "the workflow does not start from a webhook"})
        )
    );
    // Turned off.
    let mut off = flow(webhook(Some(TRIGGER_TASK), Some("Dots")), check());
    off.enabled = false;
    teams.update_workflow(&workflow.id, &off).await.unwrap();
    let answer = t.post(&workflow.id, Some(&token), None).await;
    assert_eq!(
        answer,
        (409, json!({"error": "workflow Fix is turned off"}))
    );
    // A problem: its agent step's member left the team. The body is not read.
    let agent = Step {
        id: "work".into(),
        name: "Work".into(),
        kind: StepKind::Agent {
            member: t.other.id.clone(),
            job: "Fix it.".into(),
            can_send_back_to: vec![],
            outcomes: vec![],
            delegate: None,
        },
    };
    teams
        .update_workflow(
            &workflow.id,
            &flow(webhook(Some(TRIGGER_TASK), Some("Dots")), agent),
        )
        .await
        .unwrap();
    teams
        .update(
            &t.team,
            &TeamSpec {
                org_id: None,
                workspace_id: "default".into(),
                name: "Dots management".into(),
                description: String::new(),
                members: vec![member(&t.dev)],
            },
        )
        .await
        .unwrap();
    let problem = teams
        .get_workflow(&workflow.id)
        .await
        .unwrap()
        .problem
        .unwrap();
    let answer = t.post(&workflow.id, Some(&token), Some("not json")).await;
    assert_eq!(answer.0, 409);
    assert_eq!(
        error(&answer),
        format!("workflow Fix cannot start: {problem}")
    );
    assert_eq!(t.runs().await, 0);
    t.rt.shutdown();
}

#[tokio::test]
async fn a_new_token_replaces_the_old_one_and_shows_on_the_workflow() {
    let t = Rt::start().await;
    let workflow = t.workflow(webhook(Some(TRIGGER_TASK), Some("Dots"))).await;
    assert!(!workflow.has_webhook_token);
    let mut events = t.rt.subscribe();
    let first =
        t.rt.teams()
            .regenerate_workflow_token(&workflow.id)
            .await
            .unwrap();
    // Announced, so a page showing the workflow reads it again.
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Ok(dots_core::events::RuntimeEvent::WorkflowUpdated { workflow_id, .. }) =
                events.recv().await
            {
                break workflow_id;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(event, workflow.id);
    let stored = t.rt.teams().get_workflow(&workflow.id).await.unwrap();
    assert!(stored.has_webhook_token);
    // Only its hash is kept.
    let json = serde_json::to_value(&stored).unwrap();
    assert_eq!(json["has_webhook_token"], json!(true));
    assert!(!json.to_string().contains(&first));
    t.started(t.post(&workflow.id, Some(&first), None).await)
        .await;

    let second =
        t.rt.teams()
            .regenerate_workflow_token(&workflow.id)
            .await
            .unwrap();
    assert_ne!(first, second);
    assert_eq!(t.post(&workflow.id, Some(&first), None).await.0, 401);
    t.started(t.post(&workflow.id, Some(&second), None).await)
        .await;
    // Saving the workflow keeps its token.
    let saved =
        t.rt.teams()
            .update_workflow(
                &workflow.id,
                &flow(webhook(Some("Another task."), Some("Dots")), check()),
            )
            .await
            .unwrap();
    assert!(saved.has_webhook_token);
    let wr = t
        .started(t.post(&workflow.id, Some(&second), None).await)
        .await;
    assert_eq!(wr.task, "Another task.");
    assert!(matches!(
        t.rt.teams()
            .regenerate_workflow_token("no-such-workflow")
            .await,
        Err(dots_core::Error::NotFound(_))
    ));
    t.rt.shutdown();
}
