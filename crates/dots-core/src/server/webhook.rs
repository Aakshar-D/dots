use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::ServerState;
use crate::model::{NewRun, TriggerKind};
use crate::util::ct_eq;
use crate::Error;

#[derive(Deserialize)]
pub struct TokenQuery {
    token: Option<String>,
}

pub fn bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    value.strip_prefix("Bearer ").map(|t| t.trim().to_string())
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

pub async fn trigger(
    State(s): State<ServerState>,
    Path(id): Path<String>,
    Query(q): Query<TokenQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let dot = match s.store.get_dot(&id).await {
        Ok(d) => d,
        Err(Error::NotFound(_)) => return error(StatusCode::NOT_FOUND, "unknown dot"),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let token = bearer(&headers).or(q.token);
    if !token
        .as_deref()
        .is_some_and(|t| ct_eq(t, &dot.webhook_token))
    {
        return error(StatusCode::UNAUTHORIZED, "invalid token");
    }
    if !dot.spec.enabled {
        return error(StatusCode::CONFLICT, "dot is disabled");
    }
    let payload = if body.iter().all(u8::is_ascii_whitespace) {
        None
    } else {
        match serde_json::from_slice::<Value>(&body) {
            Ok(v) => Some(v),
            Err(e) => return error(StatusCode::BAD_REQUEST, &format!("body must be JSON: {e}")),
        }
    };
    let mut new = NewRun::new(&dot.id, TriggerKind::Webhook);
    new.payload = payload;
    match s.runner.enqueue(new).await {
        Ok(run) => (StatusCode::ACCEPTED, Json(json!({ "run_id": run.id }))).into_response(),
        Err(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}
