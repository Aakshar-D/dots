//! The MCP gateway of Claude-engine runs (MCP/skills spec section 5): `POST /mcp/s/<server>`
//! is one MCP server for the CLI, backed by the run's open session with that server. The run
//! is found by its secret, as for `/mcp`. Each `tools/call` goes through the same check as a
//! local-engine call (`PermissionGate::check_mcp`), waits for an approval when it asks, and is
//! then sent to the server; a call that is not performed is a tool result with `isError`.

use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::mcp::{initialize_result, read_request, rpc_error};
use super::webhook::bearer;
use super::ServerState;
use crate::engine::{PermissionGate, PermissionOutcome};
use crate::mcp::CALL_TIMEOUT;
use crate::run_tools::RunServer;

/// How long before the CLI gives up on a call (`MCP_TOOL_TIMEOUT`) the gateway stops waiting
/// for the server, so its answer still reaches the CLI.
const ANSWER_MARGIN: Duration = Duration::from_secs(2);

pub async fn handle(
    State(s): State<ServerState>,
    Path(server): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(run_id) = bearer(&headers).and_then(|t| s.runner.run_for_secret(&t)) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(tools) = s.runner.run_tools(&run_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(open) = tools.server(&server) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let (id, req) = match read_request(&body) {
        Ok(read) => read,
        Err(answer) => return *answer,
    };
    let method = req
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let result = match method {
        "initialize" => initialize_result(&req, open.name()),
        "ping" => json!({}),
        "tools/list" => {
            // A list change the server announced is listed first.
            open.settled().await;
            json!({ "tools": open.tools() })
        }
        "tools/call" => match call(&s, &run_id, open, &req).await {
            Ok(v) => v,
            Err((code, msg)) => return rpc_error(id, code, &msg),
        },
        other => return rpc_error(id, -32601, &format!("method not found: {other}")),
    };
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

/// A tool result that says why the call was not performed (or how it failed), in the words the
/// local engine gives its model.
fn not_performed(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

/// Checks the call of `req`, waits for its approval if it asks, and sends it to the server.
/// The server's result is returned as it came.
async fn call(
    s: &ServerState,
    run_id: &str,
    server: &RunServer,
    req: &Value,
) -> Result<Value, (i64, String)> {
    let started = Instant::now();
    let params = req
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let tool = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or((-32602, "params.name is required".to_string()))?;
    let input = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => return Err((-32602, "params.arguments must be an object".to_string())),
    };
    let qualified = format!("mcp__{}__{tool}", server.name());
    // A list change announced with an earlier result holds the server before this check.
    server.settled().await;
    let checked = s
        .hub
        .check_mcp(
            run_id,
            &qualified,
            input,
            server.trust(tool),
            server.accepted_fingerprint(),
        )
        .await;
    let input = match checked {
        Ok(PermissionOutcome::Allow { input }) => input,
        Ok(PermissionOutcome::Deny { message }) => return Ok(not_performed(&message)),
        Err(e) => return Ok(not_performed(&format!("permission check failed: {e}"))),
    };
    let limit = cli_limit(s, run_id)
        .await
        .saturating_sub(started.elapsed())
        .max(Duration::from_secs(1));
    let cancel = s.runner.run_cancel(run_id).unwrap_or_default();
    Ok(
        match server
            .session()
            .call_tool(tool, input, limit, &cancel)
            .await
        {
            Ok(result) => result,
            Err(e) => not_performed(&e.to_string()),
        },
    )
}

/// How long the CLI waits for a call of this run: `MCP_TOOL_TIMEOUT`, which is the dot's
/// `approval_wait_secs` + 30 s, less the margin; the local engine's limit when the dot cannot
/// be read.
async fn cli_limit(s: &ServerState, run_id: &str) -> Duration {
    let wait = match s.store.get_run(run_id).await {
        Ok(run) => s
            .store
            .get_dot(&run.dot_id)
            .await
            .map(|dot| dot.spec.approval_wait_secs),
        Err(e) => Err(e),
    };
    match wait {
        Ok(wait) => Duration::from_secs(wait + 30).saturating_sub(ANSWER_MARGIN),
        Err(_) => CALL_TIMEOUT,
    }
}
