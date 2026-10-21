//! The built-in browser of a Claude-engine run (browser spec section 2.2): `POST
//! /mcp/s/browser` is one MCP server for the CLI. It lists the pinned Playwright tools without
//! starting anything, and hands each call to the run's browser, whose gate decides it
//! (`RunBrowser::call`). A call that is not performed is a tool result with `isError`.

use std::time::{Duration, Instant};

use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::gateway::cli_limit;
use super::mcp::{initialize_result, read_request, rpc_error};
use super::ServerState;
use crate::browser::{self, RunBrowser};

pub(super) async fn handle(
    s: &ServerState,
    run_id: &str,
    browser: &RunBrowser,
    body: &[u8],
) -> Response {
    let started = Instant::now();
    let (id, req) = match read_request(body) {
        Ok(read) => read,
        Err(answer) => return *answer,
    };
    let method = req
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let result = match method {
        "initialize" => initialize_result(&req, browser::SERVER),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": browser::tools() }),
        "tools/call" => {
            let Some(params) = req.get("params") else {
                return rpc_error(id, -32602, "missing params");
            };
            let Some(tool) = params.get("name").and_then(Value::as_str) else {
                return rpc_error(id, -32602, "params.name is required");
            };
            let input = match params.get("arguments") {
                None | Some(Value::Null) => json!({}),
                Some(args @ Value::Object(_)) => args.clone(),
                Some(_) => return rpc_error(id, -32602, "params.arguments must be an object"),
            };
            let limit = cli_limit(s, run_id)
                .await
                .saturating_sub(started.elapsed())
                .max(Duration::from_secs(1));
            let cancel = s.runner.run_cancel(run_id).unwrap_or_default();
            browser
                .call(s.hub.as_ref(), tool, input, limit, &cancel)
                .await
        }
        other => return rpc_error(id, -32601, &format!("method not found: {other}")),
    };
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}
