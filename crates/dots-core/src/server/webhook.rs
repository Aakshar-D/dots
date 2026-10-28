use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::ServerState;
use crate::model::{NewRun, TriggerKind};
use crate::Error;

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
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let dot = match s.store.get_dot(&id).await {
        Ok(d) => d,
        Err(Error::NotFound(_)) => return error(StatusCode::NOT_FOUND, "unknown dot"),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    // A bearer header only: a token in the URL ends up in proxy logs (env vault spec §11).
    let valid = match bearer(&headers) {
        Some(token) => match s.store.check_webhook_token(&dot.id, &token).await {
            Ok(valid) => valid,
            Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        },
        None => false,
    };
    if !valid {
        return error(StatusCode::UNAUTHORIZED, "invalid token");
    }
    if !dot.spec.enabled {
        return error(StatusCode::CONFLICT, "dot is disabled");
    }
    // A role that may not run the account's dots is refused now, with the refusal's words: a
    // queued run would only be held, and fire with every other call once the role returns
    // (members spec §6.3).
    match s.runner.require_run(&dot).await {
        Ok(()) => {}
        Err(Error::NotAllowed(text)) => return error(StatusCode::FORBIDDEN, &text),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
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
