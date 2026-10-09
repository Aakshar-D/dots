use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::webhook::bearer;
use super::ServerState;
use crate::engine::{PermissionGate, PermissionOutcome};

const DEFAULT_PROTOCOL: &str = "2025-06-18";

fn rpc_error(id: Value, code: i64, message: &str) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }))
        .into_response()
}

fn approve_tool() -> Value {
    json!({
        "name": "approve",
        "description": "Permission gate for dots runs. Decides whether a tool call may proceed.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "tool_name": { "type": "string" },
                "input": { "type": "object" },
                "tool_use_id": { "type": "string" }
            },
            "required": ["tool_name", "input"]
        }
    })
}

pub async fn handle(State(s): State<ServerState>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(run_id) = bearer(&headers).and_then(|t| s.runner.run_for_secret(&t)) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let req: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return rpc_error(Value::Null, -32700, &format!("parse error: {e}")),
    };
    if req.is_array() {
        return rpc_error(Value::Null, -32600, "batch requests are not supported");
    }
    let Some(id) = req.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let method = req
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let result = match method {
        "initialize" => json!({
            "protocolVersion": req
                .pointer("/params/protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_PROTOCOL),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "dots", "version": env!("CARGO_PKG_VERSION") }
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": [approve_tool()] }),
        "tools/call" => match call_approve(&s, &run_id, &req).await {
            Ok(v) => v,
            Err((code, msg)) => return rpc_error(id, code, &msg),
        },
        other => return rpc_error(id, -32601, &format!("method not found: {other}")),
    };
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

async fn call_approve(s: &ServerState, run_id: &str, req: &Value) -> Result<Value, (i64, String)> {
    let params = req
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    if params.get("name").and_then(Value::as_str) != Some("approve") {
        return Err((-32602, "unknown tool".to_string()));
    }
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    let tool = args
        .get("tool_name")
        .and_then(Value::as_str)
        .ok_or((-32602, "arguments.tool_name is required".to_string()))?;
    let input = args.get("input").cloned().unwrap_or_else(|| json!({}));
    let decision = match s.hub.check(run_id, tool, input).await {
        Ok(PermissionOutcome::Allow { input }) => {
            json!({ "behavior": "allow", "updatedInput": input })
        }
        Ok(PermissionOutcome::Deny { message }) => {
            json!({ "behavior": "deny", "message": message })
        }
        Err(e) => {
            json!({ "behavior": "deny", "message": format!("dots permission check failed: {e}") })
        }
    };
    Ok(json!({ "content": [{ "type": "text", "text": decision.to_string() }] }))
}
