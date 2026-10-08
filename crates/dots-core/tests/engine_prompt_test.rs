mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{Local, TimeZone};
use common::{fake_dot, fake_run};
use dots_core::engine::scripted::{ScriptedEngine, Step};
use dots_core::engine::{Engine, EngineEvent, PermissionGate, PermissionOutcome, RunContext};
use dots_core::model::{TriggerKind, WorkspaceMode};
use dots_core::prompt::build_prompt;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

fn ctx(cancel: CancellationToken) -> RunContext {
    RunContext {
        run_id: "RUN1".into(),
        dot: fake_dot("d", Path::new("C:/repo"), WorkspaceMode::Folder),
        workspace: "C:/repo".into(),
        prompt: "hi".into(),
        session_id: None,
        mcp_url: "http://127.0.0.1:1/mcp".into(),
        mcp_secret: "s".into(),
        cancel,
    }
}

#[test]
fn engine_event_serializes_with_kind_tag() {
    let e = EngineEvent::ToolResult {
        id: "t".into(),
        output: "o".into(),
        is_error: true,
    };
    assert_eq!(e.kind(), "tool_result");
    let v = serde_json::to_value(&e).unwrap();
    assert_eq!(v["kind"], "tool_result");
    assert_eq!(v["is_error"], true);
}

#[tokio::test]
async fn scripted_engine_emits_in_order_then_closes() {
    let engine = ScriptedEngine::new(vec![
        Step::Emit(EngineEvent::SessionStarted {
            session_id: "s1".into(),
        }),
        Step::Sleep(Duration::from_millis(5)),
        Step::Emit(EngineEvent::Finished {
            summary: "done".into(),
        }),
    ]);
    let mut rx = engine.start(ctx(CancellationToken::new())).await.unwrap();
    assert_eq!(
        rx.recv().await,
        Some(EngineEvent::SessionStarted {
            session_id: "s1".into()
        })
    );
    assert_eq!(
        rx.recv().await,
        Some(EngineEvent::Finished {
            summary: "done".into()
        })
    );
    assert_eq!(rx.recv().await, None);
}

#[tokio::test]
async fn scripted_engine_stops_on_cancel() {
    let engine = ScriptedEngine::new(vec![Step::WaitForCancel]);
    let cancel = CancellationToken::new();
    let mut rx = engine.start(ctx(cancel.clone())).await.unwrap();
    cancel.cancel();
    let closed = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .unwrap();
    assert_eq!(closed, None);
}

struct DenyAll;

#[async_trait]
impl PermissionGate for DenyAll {
    async fn check(
        &self,
        _run: &str,
        tool: &str,
        _input: Value,
    ) -> dots_core::Result<PermissionOutcome> {
        Ok(PermissionOutcome::Deny {
            message: format!("no {tool}"),
        })
    }
}

#[tokio::test]
async fn scripted_ask_reports_gate_outcome() {
    let engine = ScriptedEngine::with_gate(
        vec![Step::Ask {
            tool: "Bash".into(),
            input: json!({"command": "git push"}),
        }],
        Arc::new(DenyAll),
    );
    let mut rx = engine.start(ctx(CancellationToken::new())).await.unwrap();
    assert!(
        matches!(rx.recv().await, Some(EngineEvent::ToolCall { ref tool, .. }) if tool == "Bash")
    );
    assert_eq!(
        rx.recv().await,
        Some(EngineEvent::ToolResult {
            id: "tool-0".into(),
            output: "no Bash".into(),
            is_error: true
        })
    );
}

fn at() -> chrono::DateTime<Local> {
    Local.with_ymd_and_hms(2026, 10, 8, 9, 30, 0).unwrap()
}

#[test]
fn manual_prompt_has_instructions_trigger_and_rules() {
    let dot = fake_dot("d", Path::new("C:/repo"), WorkspaceMode::Folder);
    let run = fake_run(TriggerKind::Manual, None, None);
    let p = build_prompt(&dot, &run, Path::new("C:/ws/RUN1"), at());
    assert!(p.starts_with("Do the thing."));
    assert!(p.contains("- kind: manual"));
    assert!(p.contains("2026-10-08 09:30:00"));
    assert!(p.contains("- payload: none"));
    assert!(p.contains("C:/ws/RUN1"));
    assert!(p.contains("do not retry it"));
}

#[test]
fn webhook_prompt_includes_pretty_payload() {
    let dot = fake_dot("d", Path::new("C:/repo"), WorkspaceMode::Folder);
    let run = fake_run(TriggerKind::Webhook, Some(json!({"case": 42})), None);
    let p = build_prompt(&dot, &run, Path::new("C:/ws"), at());
    assert!(p.contains("```json\n{\n  \"case\": 42\n}\n```"));
}

#[test]
fn resume_and_chat_follow_up_send_only_the_message() {
    let dot = fake_dot("d", Path::new("C:/repo"), WorkspaceMode::Folder);
    let resume = fake_run(
        TriggerKind::Resume,
        Some(json!({"message": "Approval granted."})),
        Some("s"),
    );
    assert_eq!(
        build_prompt(&dot, &resume, Path::new("C:/ws"), at()),
        "Approval granted."
    );
    let follow = fake_run(
        TriggerKind::Chat,
        Some(json!({"message": "and now?"})),
        Some("s"),
    );
    assert_eq!(
        build_prompt(&dot, &follow, Path::new("C:/ws"), at()),
        "and now?"
    );
}

#[test]
fn first_chat_message_includes_instructions() {
    let dot = fake_dot("d", Path::new("C:/repo"), WorkspaceMode::Folder);
    let run = fake_run(TriggerKind::Chat, Some(json!({"message": "status?"})), None);
    let p = build_prompt(&dot, &run, Path::new("C:/ws"), at());
    assert!(p.starts_with("Do the thing."));
    assert!(p.contains("## Message from the user\n\nstatus?"));
    assert!(!p.contains("## Trigger"));
}
