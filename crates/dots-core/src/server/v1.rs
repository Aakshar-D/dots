//! The `/v1` routes (accounts spec §10): a caller with an API key lists the dots of the key's
//! workspace, triggers one, and reads a run's status. Every request checks the key (live, its
//! workspace still there) and the person's current role in that workspace; a dot or run of any
//! other workspace or account is answered like one that does not exist.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use super::webhook::{bearer, error, payload};
use super::ServerState;
use crate::accounts::api_keys::looks_like_key;
use crate::accounts::{Permission, PERSONAL};
use crate::model::{Dot, NewRun, TriggerKind};
use crate::store::ApiKeyRow;
use crate::Error;

/// One text for a missing, garbled, unknown or revoked key, so a caller can't probe which.
const BAD_KEY: &str = "That API key isn't valid. Make a new one in Dots.";
/// A dot or run outside the key's workspace is answered like an unknown one.
const NOT_FOUND: &str = "Not found.";

/// The key of a request, once it is known to be live and the role allows what it asks.
struct Caller {
    key: ApiKeyRow,
}

impl Caller {
    /// The account of the key's workspace; `None` is Personal.
    fn org_id(&self) -> Option<&str> {
        (self.key.scope != PERSONAL).then_some(self.key.scope.as_str())
    }

    /// Whether `dot` is in the key's account and workspace.
    fn reaches(&self, dot: &Dot) -> bool {
        dot.org_id.as_deref() == self.org_id() && dot.workspace_id == self.key.workspace_id
    }
}

fn internal(e: Error) -> Response {
    error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
}

/// The request's key, when it has the shape of a key, is known, is not revoked and its
/// workspace is still there (else 401), and the person's role in that workspace allows `p` at
/// this moment (else 403 with the refusal's words).
async fn caller(s: &ServerState, headers: &HeaderMap, p: Permission) -> Result<Caller, Response> {
    let unauthorized = || error(StatusCode::UNAUTHORIZED, BAD_KEY);
    let Some(text) = bearer(headers).filter(|t| looks_like_key(t)) else {
        return Err(unauthorized());
    };
    let key = match s.store.api_key_by_secret(&text).await {
        Ok(Some(key)) if key.revoked_at.is_none() => key,
        Ok(_) => return Err(unauthorized()),
        Err(e) => return Err(internal(e)),
    };
    match s.store.workspace(&key.scope, &key.workspace_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return Err(unauthorized()),
        Err(e) => return Err(internal(e)),
    }
    let caller = Caller { key };
    match s
        .runner
        .require_in(caller.org_id(), &caller.key.workspace_id, p)
        .await
    {
        Ok(()) => Ok(caller),
        Err(Error::NotAllowed(text)) => Err(error(StatusCode::FORBIDDEN, &text)),
        Err(e) => Err(internal(e)),
    }
}

/// `answer`, after noting that the key was used when the request succeeded.
async fn used(s: &ServerState, caller: &Caller, answer: Response) -> Response {
    if answer.status().is_success() {
        if let Err(e) = s
            .store
            .touch_api_key(&caller.key.id, &crate::util::now())
            .await
        {
            tracing::warn!(key = %caller.key.id, "noting an API key's use failed: {e}");
        }
    }
    answer
}

/// The dot `id` when the caller reaches it; 404 for an unknown dot and one it does not reach.
async fn reached_dot(s: &ServerState, caller: &Caller, id: &str) -> Result<Dot, Response> {
    match s.store.get_dot(id).await {
        Ok(dot) if caller.reaches(&dot) => Ok(dot),
        Ok(_) | Err(Error::NotFound(_)) => Err(error(StatusCode::NOT_FOUND, NOT_FOUND)),
        Err(e) => Err(internal(e)),
    }
}

/// `GET /v1/dots`: the dots of the key's workspace.
pub async fn list_dots(State(s): State<ServerState>, headers: HeaderMap) -> Response {
    let caller = match caller(&s, &headers, Permission::View).await {
        Ok(c) => c,
        Err(answer) => return answer,
    };
    let dots = match s.store.list_dots().await {
        Ok(dots) => dots,
        Err(e) => return internal(e),
    };
    let dots: Vec<_> = dots
        .iter()
        .filter(|d| caller.reaches(d))
        .map(|d| {
            json!({
                "id": d.id,
                "name": d.spec.name,
                "enabled": d.spec.enabled,
                "org_id": d.org_id,
                "workspace_id": d.workspace_id,
            })
        })
        .collect();
    let answer = (StatusCode::OK, Json(json!({ "dots": dots }))).into_response();
    used(&s, &caller, answer).await
}

/// `POST /v1/dots/{id}/trigger`: queues a run of the dot with the body as its payload, like
/// the dot's webhook.
pub async fn trigger(
    State(s): State<ServerState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let caller = match caller(&s, &headers, Permission::RunDots).await {
        Ok(c) => c,
        Err(answer) => return answer,
    };
    let dot = match reached_dot(&s, &caller, &id).await {
        Ok(dot) => dot,
        Err(answer) => return answer,
    };
    if !dot.spec.enabled {
        return error(StatusCode::CONFLICT, "dot is disabled");
    }
    let mut new = NewRun::new(&dot.id, TriggerKind::Api);
    new.payload = match payload(&body) {
        Ok(p) => p,
        Err(answer) => return *answer,
    };
    let answer = match s.runner.enqueue(new).await {
        Ok(run) => (StatusCode::ACCEPTED, Json(json!({ "run_id": run.id }))).into_response(),
        Err(e) => internal(e),
    };
    used(&s, &caller, answer).await
}

/// `GET /v1/runs/{id}`: a run's status (§15 leaves events open; status is the minimum).
pub async fn run(
    State(s): State<ServerState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let caller = match caller(&s, &headers, Permission::View).await {
        Ok(c) => c,
        Err(answer) => return answer,
    };
    let run = match s.store.get_run(&id).await {
        Ok(run) => run,
        Err(Error::NotFound(_)) => return error(StatusCode::NOT_FOUND, NOT_FOUND),
        Err(e) => return internal(e),
    };
    if let Err(answer) = reached_dot(&s, &caller, &run.dot_id).await {
        return answer;
    }
    let answer = (
        StatusCode::OK,
        Json(json!({
            "id": run.id,
            "dot_id": run.dot_id,
            "status": run.status.as_str(),
            "queued_at": run.queued_at,
            "started_at": run.started_at,
            "ended_at": run.ended_at,
            "summary": run.summary,
            "error": run.error,
        })),
    )
        .into_response();
    used(&s, &caller, answer).await
}
