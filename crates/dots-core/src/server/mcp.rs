use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use super::gateway::not_performed;
use super::webhook::bearer;
use super::ServerState;
use crate::apps::{RunApps, APP_POLICY};
use crate::checkins::Tool;
use crate::context;
use crate::engine::local::tools::cap_head;
use crate::engine::{PermissionGate, PermissionOutcome};
use crate::making::tools::Tool as MakingTool;
use crate::memory::{self, Tool as MemoryTool};
use crate::team_tools::{self, Tool as TeamTool};

const DEFAULT_PROTOCOL: &str = "2025-06-18";

pub(super) fn rpc_error(id: Value, code: i64, message: &str) -> Response {
    Json(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }))
        .into_response()
}

/// The JSON-RPC request in `body` and its id. `Err` is the answer when there is nothing to
/// answer with a result: `202` for a notification, an error for a body that is not one request.
pub(super) fn read_request(body: &[u8]) -> Result<(Value, Value), Box<Response>> {
    let req: Value = serde_json::from_slice(body)
        .map_err(|e| Box::new(rpc_error(Value::Null, -32700, &format!("parse error: {e}"))))?;
    if req.is_array() {
        return Err(Box::new(rpc_error(
            Value::Null,
            -32600,
            "batch requests are not supported",
        )));
    }
    match req.get("id").cloned() {
        Some(id) => Ok((id, req)),
        None => Err(Box::new(StatusCode::ACCEPTED.into_response())),
    }
}

/// The `initialize` result of a tools-only server named `name`, in the client's protocol
/// version.
pub(super) fn initialize_result(req: &Value, name: &str) -> Value {
    json!({
        "protocolVersion": req
            .pointer("/params/protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or(DEFAULT_PROTOCOL),
        "capabilities": { "tools": {} },
        "serverInfo": { "name": name, "version": env!("CARGO_PKG_VERSION") }
    })
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

/// The `context` tool of a run that has context collections (context library spec section 4):
/// the same arguments as the local engine's.
fn context_tool() -> Value {
    json!({
        "name": "context",
        "description": context::DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": context::properties(),
            "required": ["collection"]
        }
    })
}

/// The `graph` tool of a run whose dot uses the project graph (project graph spec section 8):
/// the same arguments as the local engine's.
fn graph_tool() -> Value {
    json!({
        "name": crate::graph::TOOL,
        "description": crate::graph::query::DESCRIPTION,
        "inputSchema": {
            "type": "object",
            "properties": crate::graph::query::properties(),
            "required": ["action"]
        },
        "annotations": { "readOnlyHint": true }
    })
}

pub async fn handle(State(s): State<ServerState>, headers: HeaderMap, body: Bytes) -> Response {
    let Some(run_id) = bearer(&headers).and_then(|t| s.runner.run_for_secret(&t)) else {
        return StatusCode::UNAUTHORIZED.into_response();
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
        "initialize" => initialize_result(&req, "dots"),
        "ping" => json!({}),
        "tools/list" => {
            let mut tools = vec![approve_tool()];
            if has_contexts(&s, &run_id) {
                tools.push(context_tool());
            }
            if s.runner
                .run_tools(&run_id)
                .is_some_and(|tools| tools.graph().is_some())
            {
                tools.push(graph_tool());
            }
            // Every run checks in (check-ins spec section 2), once the runtime has given the
            // runner the check-ins.
            if s.runner.checkins().is_some() {
                tools.extend(Tool::ALL.map(Tool::mcp_schema));
                tools.extend(MemoryTool::ALL.map(MemoryTool::mcp_schema));
            }
            // `propose_app` (app library spec section 6.2), once the runtime has given the
            // runner the apps.
            if s.runner.apps().is_some() {
                tools.push(crate::apps::propose::mcp_schema());
                tools.push(crate::apps::propose::copy_mcp_schema());
            }
            // The run's app tools (app library spec section 6).
            if let Some(apps) = run_apps(&s, &run_id) {
                tools.extend(apps.mcp_schemas());
            }
            // The workflow tools the run has (teams spec section 7): `start_workflow` for a chat
            // run of a member that may start a workflow, a step's tools for a run of its step.
            tools.extend(
                TeamTool::ALL
                    .into_iter()
                    .filter(|t| offers(&s, &run_id, *t))
                    .map(TeamTool::mcp_schema),
            );
            // A run of a chat the person started (conversational chat spec section 4).
            if has_instructions(&s, &run_id) {
                tools.push(crate::instructions::mcp_schema());
            }
            // A run of a dot that may make dots (dots-make-dots design section 5).
            if let Some(limits) = making(&s, &run_id) {
                tools.extend(MakingTool::ALL.map(|tool| tool.mcp_schema(&limits)));
            }
            json!({ "tools": tools })
        }
        "tools/call" => {
            let name = req.pointer("/params/name").and_then(Value::as_str);
            let answer = match (name, name.and_then(Tool::from_name)) {
                (Some("context"), _) => call_context(&s, &run_id, &req).await,
                (Some(crate::graph::TOOL), _) => call_graph(&s, &run_id, &req).await,
                (Some(crate::apps::propose::TOOL), _) => call_propose(&s, &run_id, &req).await,
                (Some(crate::apps::propose::COPY_TOOL), _) => call_copy(&s, &run_id, &req).await,
                (Some(crate::instructions::TOOL), _) => call_instructions(&s, &run_id, &req).await,
                (_, Some(tool)) => call_check_in(&s, &run_id, &req, tool).await,
                (Some(n), None) if TeamTool::from_name(n).is_some() => {
                    let tool = TeamTool::from_name(n).expect("a workflow tool");
                    call_workflow(&s, &run_id, &req, tool).await
                }
                (Some(n), None) if MemoryTool::from_name(n).is_some() => {
                    let tool = MemoryTool::from_name(n).expect("a memory tool");
                    call_memory(&s, &run_id, &req, tool).await
                }
                (Some(n), None)
                    if run_apps(&s, &run_id).is_some_and(|a| a.resolve(n).is_some()) =>
                {
                    call_app(&s, &run_id, &req, n).await
                }
                (Some(n), None) if MakingTool::from_name(n).is_some() => {
                    let tool = MakingTool::from_name(n).expect("a tool for making dots");
                    call_making(&s, &run_id, &req, tool).await
                }
                _ => call_approve(&s, &run_id, &req).await,
            };
            match answer {
                Ok(v) => v,
                Err((code, msg)) => return rpc_error(id, code, &msg),
            }
        }
        other => return rpc_error(id, -32601, &format!("method not found: {other}")),
    };
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

/// The run may propose new instructions, so `propose_instructions` is one of its tools.
fn has_instructions(s: &ServerState, run_id: &str) -> bool {
    s.runner
        .run_tools(run_id)
        .is_some_and(|tools| tools.instructions())
}

/// Checks a `propose_instructions` call against the dot's policy under `Instructions` and
/// keeps the proposal (conversational chat spec section 4). A call that is not performed is a
/// tool result with `isError`, as for the check-in tools.
async fn call_instructions(
    s: &ServerState,
    run_id: &str,
    req: &Value,
) -> Result<Value, (i64, String)> {
    let Some(checkins) = s.runner.checkins().filter(|_| has_instructions(s, run_id)) else {
        return Err((-32602, "unknown tool".to_string()));
    };
    let params = req
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let mut input = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => return Err((-32602, "params.arguments must be an object".to_string())),
    };
    // Only the tool's own arguments, as the local engine's `prepare` keeps.
    if let Value::Object(args) = &mut input {
        args.retain(|key, _| crate::instructions::REQUIRED.contains(&key.as_str()));
    }
    let input = match s
        .hub
        .check(run_id, crate::instructions::POLICY_NAME, input)
        .await
    {
        Ok(PermissionOutcome::Allow { input }) => input,
        Ok(PermissionOutcome::Deny { message }) => return Ok(not_performed(&message)),
        Err(e) => return Ok(not_performed(&format!("permission check failed: {e}"))),
    };
    let arg = |key: &str| input.get(key).and_then(Value::as_str).unwrap_or_default();
    Ok(
        match checkins
            .instructions()
            .propose(run_id, arg("instructions"), arg("reason"))
            .await
        {
            Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
            Err(text) => not_performed(&text),
        },
    )
}

/// The maker's limits, when the run has the tools for making dots (`RunTools::making`).
fn making(s: &ServerState, run_id: &str) -> Option<crate::model::Making> {
    s.runner
        .run_tools(run_id)
        .and_then(|tools| tools.making().copied())
}

/// Checks a call of the tool for making dots `tool` against the dot's policy under `MakeDots`,
/// waiting for an approval when a rule asks, and has the makers' service perform it
/// (dots-make-dots design sections 5 and 7 to 9). The policy is asked about the tool by name
/// with its arguments (`making::tools::policy_input`). Arguments the tool cannot read are
/// refused before the policy is asked, in a sentence for the model. A call that is not
/// performed, or that the service refuses, is a tool result with `isError`, as for the
/// check-in tools; a tool the run was not offered is unknown.
async fn call_making(
    s: &ServerState,
    run_id: &str,
    req: &Value,
    tool: MakingTool,
) -> Result<Value, (i64, String)> {
    let makers = s
        .runner
        .checkins()
        .filter(|_| making(s, run_id).is_some())
        .and_then(|checkins| checkins.makers());
    let Some(makers) = makers else {
        return Err((-32602, "unknown tool".to_string()));
    };
    let params = req
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let arguments = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => return Err((-32602, "params.arguments must be an object".to_string())),
    };
    if let Err(text) = crate::making::tools::check_arguments(tool, &arguments) {
        return Ok(not_performed(&text));
    }
    let input = crate::making::tools::policy_input(tool, &arguments);
    let checked = match s
        .hub
        .check(run_id, crate::making::tools::POLICY_NAME, input)
        .await
    {
        Ok(PermissionOutcome::Allow { input }) => input,
        Ok(PermissionOutcome::Deny { message }) => return Ok(not_performed(&message)),
        Err(e) => return Ok(not_performed(&format!("permission check failed: {e}"))),
    };
    Ok(
        match crate::making::tools::call(&makers, run_id, &checked).await {
            Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
            Err(text) => not_performed(&text),
        },
    )
}

/// The run has the workflow tool `tool` (`RunTools::offers`).
fn offers(s: &ServerState, run_id: &str, tool: TeamTool) -> bool {
    s.runner
        .run_tools(run_id)
        .is_some_and(|tools| tools.offers(tool))
}

/// Checks a call of the workflow tool `tool` against the dot's policy under `Workflow`, waiting
/// for an approval when a rule asks, and has `Workflows` perform it (teams spec section 7). A
/// call that is not performed is a tool result with `isError`, as for the check-in tools; a
/// tool the run does not have is unknown.
async fn call_workflow(
    s: &ServerState,
    run_id: &str,
    req: &Value,
    tool: TeamTool,
) -> Result<Value, (i64, String)> {
    let (Some(checkins), Some(tools)) = (
        s.runner.checkins(),
        s.runner
            .run_tools(run_id)
            .filter(|tools| tools.offers(tool)),
    ) else {
        return Err((-32602, "unknown tool".to_string()));
    };
    let Some(workflows) = checkins.workflows() else {
        return Err((-32602, "unknown tool".to_string()));
    };
    let params = req
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let mut input = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => return Err((-32602, "params.arguments must be an object".to_string())),
    };
    // Only the tool's own arguments, as the local engine's `prepare` keeps.
    let own = tool.properties();
    if let (Value::Object(args), Value::Object(own)) = (&mut input, &own) {
        args.retain(|key, _| own.contains_key(key));
    }
    let input = match s.hub.check(run_id, team_tools::POLICY_NAME, input).await {
        Ok(PermissionOutcome::Allow { input }) => input,
        Ok(PermissionOutcome::Deny { message }) => return Ok(not_performed(&message)),
        Err(e) => return Ok(not_performed(&format!("permission check failed: {e}"))),
    };
    let done = match (tool, tools.step()) {
        (TeamTool::StartWorkflow, _) => {
            workflows
                .call_start(run_id, tools.startable(), &input)
                .await
        }
        (TeamTool::StartMember, Some(step)) => {
            workflows.call_start_member(run_id, step, &input).await
        }
        (TeamTool::ReportOutcome, Some(step)) => {
            workflows.call_report_outcome(run_id, step, &input).await
        }
        (TeamTool::SendBack, Some(step)) => workflows.call_send_back(run_id, step, &input).await,
        // `offers` gives a step's tools only to a run that has a step.
        _ => return Err((-32602, "unknown tool".to_string())),
    };
    Ok(match done {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
        Err(text) => not_performed(&text),
    })
}

/// The run has context collections open, so `context` is one of its tools.
fn has_contexts(s: &ServerState, run_id: &str) -> bool {
    s.runner
        .run_tools(run_id)
        .is_some_and(|tools| !tools.contexts().is_empty())
}

/// Checks a `propose_app` call against the dot's policy under `ProposeApp`, waiting for an
/// approval when a rule asks, and hands the folder, confined to the run's workspace, to the
/// apps (app library spec section 6.2). A call that is not performed, or whose folder fails its
/// checks, is a tool result with `isError`.
async fn call_propose(s: &ServerState, run_id: &str, req: &Value) -> Result<Value, (i64, String)> {
    let Some(apps) = s.runner.apps() else {
        return Err((-32602, "unknown tool".to_string()));
    };
    let params = req
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let mut input = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => return Err((-32602, "params.arguments must be an object".to_string())),
    };
    // Only the tool's own arguments, as for the memory tools: the approval names what will run.
    if let Value::Object(args) = &mut input {
        args.retain(|key, _| crate::apps::propose::REQUIRED.contains(&key.as_str()));
    }
    let input = match s
        .hub
        .check(run_id, crate::apps::PROPOSE_POLICY, input)
        .await
    {
        Ok(PermissionOutcome::Allow { input }) => input,
        Ok(PermissionOutcome::Deny { message }) => return Ok(not_performed(&message)),
        Err(e) => return Ok(not_performed(&format!("permission check failed: {e}"))),
    };
    let workspace = match s.store.get_run(run_id).await {
        Ok(run) => run.workspace_path,
        Err(e) => return Ok(not_performed(&format!("the run could not be read: {e}"))),
    };
    let Some(workspace) = workspace else {
        return Ok(not_performed("this run has no workspace"));
    };
    let path = input["path"].as_str().unwrap_or_default();
    let folder = match crate::engine::local::tools::confine(std::path::Path::new(&workspace), path)
    {
        Ok(folder) => folder,
        Err(e) => return Ok(not_performed(&e)),
    };
    let note = input["note"].as_str().unwrap_or_default();
    Ok(match apps.propose(run_id, folder, note).await {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
        Err(text) => not_performed(&cap_head(text)),
    })
}

/// Checks a `copy_app` call against the dot's policy under `ProposeApp`, waiting for an
/// approval when a rule asks, and copies the app into the folder, confined to the run's
/// workspace (app library spec section 15); `copy_to` also judges the destination as a `Write`
/// of the dot. A call without `app` and `path` as text is refused before the gate. A call that
/// is not performed is a tool result with `isError`.
async fn call_copy(s: &ServerState, run_id: &str, req: &Value) -> Result<Value, (i64, String)> {
    let Some(apps) = s.runner.apps() else {
        return Err((-32602, "unknown tool".to_string()));
    };
    let params = req
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let mut input = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => return Err((-32602, "params.arguments must be an object".to_string())),
    };
    if let Value::Object(args) = &mut input {
        args.retain(|key, _| crate::apps::propose::COPY_REQUIRED.contains(&key.as_str()));
    }
    if !(input["app"].is_string() && input["path"].is_string()) {
        return Ok(not_performed("copy_app needs \"app\" and \"path\" as text"));
    }
    let input = match s
        .hub
        .check(run_id, crate::apps::PROPOSE_POLICY, input)
        .await
    {
        Ok(PermissionOutcome::Allow { input }) => input,
        Ok(PermissionOutcome::Deny { message }) => return Ok(not_performed(&message)),
        Err(e) => return Ok(not_performed(&format!("permission check failed: {e}"))),
    };
    let workspace = match s.store.get_run(run_id).await {
        Ok(run) => run.workspace_path,
        Err(e) => return Ok(not_performed(&format!("the run could not be read: {e}"))),
    };
    let Some(workspace) = workspace else {
        return Ok(not_performed("this run has no workspace"));
    };
    let path = input["path"].as_str().unwrap_or_default();
    let folder = match crate::engine::local::tools::confine(std::path::Path::new(&workspace), path)
    {
        Ok(folder) => folder,
        Err(e) => return Ok(not_performed(&e)),
    };
    let app = input["app"].as_str().unwrap_or_default();
    Ok(match apps.copy_to(run_id, app, folder, path).await {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
        Err(text) => not_performed(&cap_head(text)),
    })
}

/// The run's apps, when it has any.
fn run_apps(s: &ServerState, run_id: &str) -> Option<std::sync::Arc<RunApps>> {
    s.runner
        .run_tools(run_id)
        .and_then(|tools| tools.apps().cloned())
}

/// Checks a call of the app tool `key` (`<app>__<tool>`) against the dot's policy under `App`,
/// waiting for an approval when a rule asks, and calls the app as the run's dot (app library
/// spec section 6). A call that is not performed, or that the app refuses, is a tool result
/// with `isError`, as for the memory tools.
async fn call_app(
    s: &ServerState,
    run_id: &str,
    req: &Value,
    key: &str,
) -> Result<Value, (i64, String)> {
    let Some(apps) = run_apps(s, run_id) else {
        return Err((-32602, "unknown tool".to_string()));
    };
    let params = req
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let input = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => return Err((-32602, "params.arguments must be an object".to_string())),
    };
    let Some(checked) = apps.policy_input(key, &input) else {
        return Err((-32602, "unknown tool".to_string()));
    };
    let checked = match s.hub.check(run_id, APP_POLICY, checked).await {
        Ok(PermissionOutcome::Allow { input }) => input,
        Ok(PermissionOutcome::Deny { message }) => return Ok(not_performed(&message)),
        Err(e) => return Ok(not_performed(&format!("permission check failed: {e}"))),
    };
    Ok(match apps.call(&checked).await {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
        Err(text) => not_performed(&text),
    })
}

/// Checks a `context` call against the dot's policy as the tool `Context`, waiting for an
/// approval when a rule asks, and reads the run's collections. A call that is not performed is
/// a tool result with `isError`, as the gateway answers.
async fn call_context(s: &ServerState, run_id: &str, req: &Value) -> Result<Value, (i64, String)> {
    let params = req
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let input = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => return Err((-32602, "params.arguments must be an object".to_string())),
    };
    let Some(tools) = s
        .runner
        .run_tools(run_id)
        .filter(|tools| !tools.contexts().is_empty())
    else {
        return Err((-32602, "unknown tool".to_string()));
    };
    let input = match s.hub.check(run_id, "Context", input).await {
        Ok(PermissionOutcome::Allow { input }) => input,
        Ok(PermissionOutcome::Deny { message }) => return Ok(not_performed(&message)),
        Err(e) => return Ok(not_performed(&format!("permission check failed: {e}"))),
    };
    let collections = tools.contexts().to_vec();
    let read = tokio::task::spawn_blocking(move || context::call(&collections, &input)).await;
    Ok(match read {
        Ok(Ok(text)) => {
            json!({ "content": [{ "type": "text", "text": cap_head(text) }] })
        }
        Ok(Err(e)) => not_performed(&cap_head(e)),
        Err(e) => not_performed(&format!("reading the collection failed: {e}")),
    })
}

/// Checks a `graph` call against the dot's policy as the tool `Graph` and answers it from the
/// project's graph (project graph spec section 8). A call that is not performed is a tool
/// result with `isError`, as for `context`.
async fn call_graph(s: &ServerState, run_id: &str, req: &Value) -> Result<Value, (i64, String)> {
    let params = req
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let mut input = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => return Err((-32602, "params.arguments must be an object".to_string())),
    };
    let Some(graph) = s
        .runner
        .run_tools(run_id)
        .and_then(|tools| tools.graph().cloned())
    else {
        return Err((-32602, "unknown tool".to_string()));
    };
    // Only the tool's own arguments.
    if let Value::Object(args) = &mut input {
        args.retain(|key, _| {
            ["action", "query", "node", "to", "path", "kind", "depth"].contains(&key.as_str())
        });
    }
    let input = match s.hub.check(run_id, crate::graph::POLICY_NAME, input).await {
        Ok(PermissionOutcome::Allow { input }) => input,
        Ok(PermissionOutcome::Deny { message }) => return Ok(not_performed(&message)),
        Err(e) => return Ok(not_performed(&format!("permission check failed: {e}"))),
    };
    Ok(
        match graph
            .graphs
            .call(&graph.project, &graph.scope, &input)
            .await
        {
            Ok(text) => json!({ "content": [{ "type": "text", "text": cap_head(text) }] }),
            Err(e) => not_performed(&cap_head(e)),
        },
    )
}

/// Checks a call of a check-in tool against the dot's policy under its policy name (`Notify`,
/// `Ask`, `FollowUp`), waiting for an approval when a rule asks, and performs it (check-ins
/// spec section 2). A call that is not performed is a tool result with `isError`, as for
/// `context`.
async fn call_check_in(
    s: &ServerState,
    run_id: &str,
    req: &Value,
    tool: Tool,
) -> Result<Value, (i64, String)> {
    let Some(checkins) = s.runner.checkins() else {
        return Err((-32602, "unknown tool".to_string()));
    };
    let params = req
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let input = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => return Err((-32602, "params.arguments must be an object".to_string())),
    };
    let input = match s.hub.check(run_id, tool.policy_name(), input).await {
        Ok(PermissionOutcome::Allow { input }) => input,
        Ok(PermissionOutcome::Deny { message }) => return Ok(not_performed(&message)),
        Err(e) => return Ok(not_performed(&format!("permission check failed: {e}"))),
    };
    Ok(match checkins.call(run_id, tool, &input).await {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
        Err(text) => not_performed(&text),
    })
}

/// Checks a call of a memory tool against the dot's policy under `Memory`, waiting for an
/// approval when a rule asks, and performs it for the run's dot (dot memory spec section 3). A
/// call that is not performed is a tool result with `isError`, as for the check-in tools.
async fn call_memory(
    s: &ServerState,
    run_id: &str,
    req: &Value,
    tool: MemoryTool,
) -> Result<Value, (i64, String)> {
    let Some(checkins) = s.runner.checkins() else {
        return Err((-32602, "unknown tool".to_string()));
    };
    let params = req
        .get("params")
        .ok_or((-32602, "missing params".to_string()))?;
    let input = match params.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(args @ Value::Object(_)) => args.clone(),
        Some(_) => return Err((-32602, "params.arguments must be an object".to_string())),
    };
    // Only the tool's own argument, as the local engine's `prepare` keeps: the approval then
    // names exactly the tool that will run, and one tool's grant cannot be redeemed by another.
    let mut input = input;
    let own = tool.properties();
    if let (Value::Object(args), Value::Object(own)) = (&mut input, &own) {
        args.retain(|key, _| own.contains_key(key));
    }
    let input = match s.hub.check(run_id, memory::POLICY_NAME, input).await {
        Ok(PermissionOutcome::Allow { input }) => input,
        Ok(PermissionOutcome::Deny { message }) => return Ok(not_performed(&message)),
        Err(e) => return Ok(not_performed(&format!("permission check failed: {e}"))),
    };
    Ok(match checkins.memories().call(run_id, tool, &input).await {
        Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
        Err(text) => not_performed(&text),
    })
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
    // A tool of the run's open sessions goes through the gateway, which checks every call
    // (`gateway.rs`): checking it here as well would ask twice.
    let gateway = s
        .runner
        .run_tools(run_id)
        .is_some_and(|tools| tools.checks_itself(tool));
    if gateway {
        let allow = json!({ "behavior": "allow", "updatedInput": input });
        return Ok(json!({ "content": [{ "type": "text", "text": allow.to_string() }] }));
    }
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
