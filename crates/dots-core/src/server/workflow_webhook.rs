//! A workflow's webhook (teams spec section 5.1): `POST /workflows/{id}/trigger` with the
//! workflow's token as a bearer starts a run of it, as a dot's webhook (`webhook.rs`) queues a
//! run of the dot. The body names the task and the project; each falls back to the webhook
//! trigger's own.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use super::webhook::{bearer, error, payload};
use super::ServerState;
use crate::model::WorkflowTrigger;
use crate::workflow::Trigger;
use crate::workflows::StartRequest;
use crate::Error;

/// What a body may ask for. Other keys are ignored, so a sender's own payload can start the
/// workflow with the trigger's task and project.
#[derive(Debug, Default, Deserialize)]
struct Asked {
    #[serde(default)]
    task: Option<String>,
    #[serde(default)]
    project: Option<String>,
}

/// The body's task and project: none asked when it is empty or blank; a 400 answer when it is
/// not JSON, not an object, or its `task` or `project` is not a string.
fn asked(body: &[u8]) -> Result<Asked, Box<Response>> {
    let Some(value) = payload(body)? else {
        return Ok(Asked::default());
    };
    // Checked first: serde would read an array as the struct's fields in order.
    if !value.is_object() {
        return Err(Box::new(error(StatusCode::BAD_REQUEST, SHAPE)));
    }
    serde_json::from_value(value).map_err(|_| Box::new(error(StatusCode::BAD_REQUEST, SHAPE)))
}

/// The most runs of one workflow in flight (running or waiting, a wait on the person's check
/// included) that its webhook starts past: a call beyond it is refused with 429 and starts
/// nothing (the person's decision, 2026-11-04).
pub const MAX_WEBHOOK_IN_FLIGHT: u32 = 3;

/// The 429 answer of a workflow at `MAX_WEBHOOK_IN_FLIGHT`.
pub const TOO_MANY_IN_FLIGHT: &str = "too many runs in flight";

/// Held from the count of a workflow's runs in flight until its start has made the run, so two
/// calls at once cannot both pass the cap.
static STARTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The 400 answer to a body of another shape.
const SHAPE: &str = "body must be a JSON object whose \"task\" and \"project\", when given, are \
                     strings";

/// The body's value when it is not blank, else the trigger's fallback when that is not blank.
fn given(asked: Option<String>, fallback: &Option<String>) -> Option<String> {
    [asked.as_deref(), fallback.as_deref()]
        .into_iter()
        .flatten()
        .map(str::trim)
        .find(|v| !v.is_empty())
        .map(str::to_string)
}

/// The answer to a start `Workflows::start` refused: a role that may not run dots is 403, a
/// bad task or a project that cannot be worked on is 400, a workflow that cannot start now is
/// 409, with the refusal's words.
fn refused(e: Error) -> Response {
    match e {
        Error::NotAllowed(text) => error(StatusCode::FORBIDDEN, &text),
        Error::Invalid(text) => error(StatusCode::BAD_REQUEST, &text),
        Error::Conflict(text) => error(StatusCode::CONFLICT, &text),
        // Deleted since it was read.
        Error::NotFound(_) => error(StatusCode::NOT_FOUND, "unknown workflow"),
        e => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

/// Starts a run of the workflow `id` (module docs). Answers 202 `{"workflow_run_id"}`; 404
/// `unknown workflow`; 401 `invalid token` (none given, or the workflow has none); 409 for a
/// workflow without a webhook trigger, turned off or with a problem; 429 `too many runs in
/// flight` for one with `MAX_WEBHOOK_IN_FLIGHT` runs in flight; 400 for a body of another
/// shape, no task or project, or a project the team does not have; and `Workflows::start`'s
/// refusals (`refused`).
pub async fn trigger(
    State(s): State<ServerState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let workflow = match s.store.get_workflow(&id).await {
        Ok(w) => w,
        Err(Error::NotFound(_)) => return error(StatusCode::NOT_FOUND, "unknown workflow"),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    // A bearer header only, as a dot's: a token in the URL ends up in proxy logs.
    let valid = match bearer(&headers) {
        Some(token) => match s.store.check_workflow_token(&workflow.id, &token).await {
            Ok(valid) => valid,
            Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        },
        None => false,
    };
    if !valid {
        return error(StatusCode::UNAUTHORIZED, "invalid token");
    }
    let Trigger::Webhook {
        task: trigger_task,
        project: trigger_project,
    } = &workflow.spec.definition.trigger
    else {
        return error(
            StatusCode::CONFLICT,
            "the workflow does not start from a webhook",
        );
    };
    // `Workflows::start`'s words, answered before the body is read: a workflow that cannot
    // start is 409 whatever the body asks.
    let name = &workflow.spec.name;
    if !workflow.spec.enabled {
        return error(
            StatusCode::CONFLICT,
            &format!("workflow {name} is turned off"),
        );
    }
    if let Some(problem) = &workflow.problem {
        return error(
            StatusCode::CONFLICT,
            &format!("workflow {name} cannot start: {problem}"),
        );
    }
    // The cap, after the 409s and before the body is read, as they are.
    let _one = STARTS.lock().await;
    match s.store.count_workflow_runs_in_flight(&workflow.id).await {
        Ok(n) if n >= MAX_WEBHOOK_IN_FLIGHT => {
            return error(StatusCode::TOO_MANY_REQUESTS, TOO_MANY_IN_FLIGHT)
        }
        Ok(_) => {}
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
    let asked = match asked(&body) {
        Ok(a) => a,
        Err(answer) => return *answer,
    };
    let Some(task) = given(asked.task, trigger_task) else {
        return error(
            StatusCode::BAD_REQUEST,
            "no task: give \"task\" in the body, or a task in the workflow's webhook trigger",
        );
    };
    let Some(project) = given(asked.project, trigger_project) else {
        return error(
            StatusCode::BAD_REQUEST,
            "no project: give \"project\" in the body, or a project in the workflow's webhook \
             trigger",
        );
    };
    let team = match s.store.get_team(&workflow.team_id).await {
        Ok(t) => t,
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let Some(project) = crate::teams::project_named(&team, &project) else {
        return error(
            StatusCode::BAD_REQUEST,
            &format!("the team has no project named \"{project}\""),
        );
    };
    let Some(workflows) = s.runner.checkins().and_then(|c| c.workflows()) else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "workflows are not running");
    };
    match workflows
        .start(StartRequest {
            workflow_id: workflow.id.clone(),
            project_id: project.id.clone(),
            task,
            trigger: WorkflowTrigger::Webhook,
            origin_run_id: None,
            test: false,
        })
        .await
    {
        Ok(wr) => (
            StatusCode::ACCEPTED,
            Json(json!({ "workflow_run_id": wr.id })),
        )
            .into_response(),
        Err(e) => refused(e),
    }
}
