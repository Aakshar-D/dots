//! `propose_instructions` on both engines (conversational chat spec section 4): offered to and
//! named in a run of a chat the person started, never to another run or under a deny rule.

mod common;
mod llm;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{spec, wait_status};
use dots_core::instructions::{InstructionStatus, SENT};
use dots_core::model::{DotSpec, EngineKind, RunStatus, WorkspaceMode};
use dots_core::policy::{Action, Rule};
use dots_core::prompt::NO_INSTRUCTIONS_TOOL;
use dots_core::{Config, Runtime};
use llm::{call as llm_call, messages, text as llm_text, tool_result, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;

struct Rt {
    dir: TempDir,
    rt: Runtime,
}

async fn start_with(edit: impl FnOnce(&mut Config, &Path)) -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.local_retry_delays = vec![Duration::ZERO; 2];
    edit(&mut cfg, dir.path());
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { dir, rt }
}

fn local_spec(t: &Rt, endpoint: &str, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(format!("{endpoint}/v1"));
    s.model = "test-model".into();
    s
}

fn tool_names(req: &Value) -> Vec<String> {
    req["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap().to_string())
        .collect()
}

fn prompt_of(req: &Value) -> String {
    messages(req)
        .iter()
        .filter_map(|m| m["content"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[tokio::test]
async fn a_local_dot_proposes_from_the_person_s_chat_only() {
    let t = start_with(|_, _| {}).await;
    let llm = Llm::start(vec![
        llm_call(
            "c1",
            "propose_instructions",
            json!({"instructions": "Do the thing twice.", "reason": "Twice"}),
        ),
        llm_text("Sent it to you."),
        llm_text("Done."),
    ])
    .await;
    let dot =
        t.rt.create_dot(local_spec(&t, &llm.endpoint(), "shopper"), None, None)
            .await
            .unwrap();
    let chat =
        t.rt.chat(&dot.id, "do it twice from now on", None)
            .await
            .unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::Succeeded, 30).await;
    let reqs = llm.requests();
    assert!(tool_names(&reqs[0]).contains(&"propose_instructions".to_string()));
    assert!(prompt_of(&reqs[0])
        .contains("propose new instructions with `propose_instructions` instead"));
    assert_eq!(tool_result(&reqs[1], "c1"), SENT);
    let list =
        t.rt.store()
            .list_instruction_proposals(Some(&dot.id))
            .await
            .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].instructions, "Do the thing twice.");
    assert_eq!(list[0].status, InstructionStatus::Pending);
    assert_eq!(list[0].run_id, chat.id);

    // A run that is not a chat the person started is neither offered nor told about it.
    let manual = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &manual.id, RunStatus::Succeeded, 30).await;
    let reqs = llm.requests();
    assert!(!tool_names(&reqs[2]).contains(&"propose_instructions".to_string()));
    assert!(!prompt_of(&reqs[2]).contains("propose_instructions"));
    t.rt.shutdown();
}

#[tokio::test]
async fn a_deny_rule_turns_the_tool_off_and_the_chat_is_told_to_send_the_person_to_the_editor() {
    let t = start_with(|_, _| {}).await;
    let llm = Llm::start(vec![llm_text("Edit them in the editor.")]).await;
    let mut s = local_spec(&t, &llm.endpoint(), "denied");
    s.policy
        .rules
        .insert(0, Rule::new("Instructions", Action::Deny));
    let dot = t.rt.create_dot(s, None, None).await.unwrap();
    let chat =
        t.rt.chat(&dot.id, "do it twice from now on", None)
            .await
            .unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::Succeeded, 30).await;
    let reqs = llm.requests();
    assert!(!tool_names(&reqs[0]).contains(&"propose_instructions".to_string()));
    assert!(prompt_of(&reqs[0]).contains(NO_INSTRUCTIONS_TOOL.trim_end()));
    t.rt.shutdown();
}

#[tokio::test]
async fn a_claude_dot_proposes_through_dots_own_server_from_the_person_s_chat() {
    let script = json!([
        {"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}},
        {"mcp_list": "dots"},
        {"mcp_call": {"server": "dots", "tool": "propose_instructions",
                      "arguments": {"instructions": "Do the thing twice.", "reason": "Twice"}}},
        {"emit": {"type": "result", "subtype": "success", "is_error": false, "result": "done",
                  "usage": {"input_tokens": 1, "output_tokens": 1}}}
    ]);
    let t = start_with(|cfg, dir| {
        let path = dir.join("script.json");
        std::fs::write(&path, script.to_string()).unwrap();
        cfg.claude_env = vec![
            (
                "FAKE_CLAUDE_SCRIPT".into(),
                path.to_string_lossy().into_owned(),
            ),
            (
                "FAKE_CLAUDE_ARGS_OUT".into(),
                dir.join("args.json").to_string_lossy().into_owned(),
            ),
        ];
    })
    .await;
    let mut s = spec("claude");
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Claude;
    let dot = t.rt.create_dot(s, None, None).await.unwrap();
    let chat =
        t.rt.chat(&dot.id, "do it twice from now on", None)
            .await
            .unwrap();
    wait_status(t.rt.store(), &chat.id, RunStatus::Succeeded, 30).await;
    let recorded: Value =
        serde_json::from_str(&std::fs::read_to_string(t.dir.path().join("args.json")).unwrap())
            .unwrap();
    let args: Vec<String> = serde_json::from_value(recorded["args"].clone()).unwrap();
    assert!(
        args.contains(&"mcp__dots__propose_instructions".to_string()),
        "{args:?}"
    );
    assert!(recorded["prompt"]
        .as_str()
        .unwrap()
        .contains("with `mcp__dots__propose_instructions` instead"));
    let list =
        t.rt.store()
            .list_instruction_proposals(Some(&dot.id))
            .await
            .unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].reason, "Twice");
    t.rt.shutdown();
}
