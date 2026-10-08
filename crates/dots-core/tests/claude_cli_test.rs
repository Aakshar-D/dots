mod common;

use std::path::Path;
#[cfg(not(windows))]
use std::path::PathBuf;

use common::fake_dot;
use dots_core::engine::claude::args::{build_args, mcp_config, resolve_claude, resolve_in_dirs};
use dots_core::engine::claude::parse::parse_line;
use dots_core::engine::{EngineEvent, RunContext};
use dots_core::model::WorkspaceMode;
use dots_core::policy::{Policy, Preset};
use serde_json::json;
use tokio_util::sync::CancellationToken;

const INIT: &str = r#"{"type":"system","subtype":"init","cwd":"C:\\x","session_id":"84fee880-875a-47ff-835e-60eba6825534","tools":["Bash"],"model":"claude-haiku-5-5","permissionMode":"default"}"#;
const TEXT: &str = r#"{"type":"assistant","message":{"model":"claude-haiku-5-5","role":"assistant","content":[{"type":"text","text":"pong"}],"usage":{"input_tokens":2,"output_tokens":4}},"session_id":"84fee880"}"#;
const TOOL_USE: &str = r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"hm"},{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"git status","description":"Check"}}]}}"#;
const RESULT_OK: &str = r#"{"type":"result","subtype":"success","is_error":false,"result":"pong","session_id":"84fee880","usage":{"input_tokens":2,"cache_creation_input_tokens":26807,"cache_read_input_tokens":10,"output_tokens":4},"num_turns":1}"#;
const RESULT_MAX_TURNS: &str = r#"{"type":"result","subtype":"error_max_turns","is_error":true,"session_id":"x","usage":{"input_tokens":1,"output_tokens":1}}"#;
const RATE: &str = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed"}}"#;

#[test]
fn parses_init_text_and_tool_use() {
    assert_eq!(
        parse_line(INIT),
        vec![EngineEvent::SessionStarted {
            session_id: "84fee880-875a-47ff-835e-60eba6825534".into()
        }]
    );
    assert_eq!(
        parse_line(TEXT),
        vec![EngineEvent::AssistantText {
            text: "pong".into()
        }]
    );
    assert_eq!(
        parse_line(TOOL_USE),
        vec![EngineEvent::ToolCall {
            id: "toolu_1".into(),
            tool: "Bash".into(),
            input: json!({"command": "git status", "description": "Check"}),
        }]
    );
}

#[test]
fn parses_tool_results_in_both_shapes() {
    let s = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"On branch main","is_error":false}]}}"#;
    assert_eq!(
        parse_line(s),
        vec![EngineEvent::ToolResult {
            id: "toolu_1".into(),
            output: "On branch main".into(),
            is_error: false
        }]
    );
    let a = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":[{"type":"text","text":"a"},{"type":"text","text":"b"}],"is_error":true}]}}"#;
    assert_eq!(
        parse_line(a),
        vec![EngineEvent::ToolResult {
            id: "t2".into(),
            output: "a\nb".into(),
            is_error: true
        }]
    );
    let prompt_echo = r#"{"type":"user","message":{"content":"hello"}}"#;
    assert!(parse_line(prompt_echo).is_empty());
}

#[test]
fn parses_results() {
    assert_eq!(
        parse_line(RESULT_OK),
        vec![
            EngineEvent::Usage {
                tokens_in: 26819,
                tokens_out: 4
            },
            EngineEvent::Finished {
                summary: "pong".into()
            },
        ]
    );
    assert_eq!(
        parse_line(RESULT_MAX_TURNS),
        vec![
            EngineEvent::Usage {
                tokens_in: 1,
                tokens_out: 1
            },
            EngineEvent::Failed {
                error: "error_max_turns".into()
            },
        ]
    );
}

#[test]
fn unknown_and_invalid_lines_are_raw_and_blank_is_nothing() {
    assert_eq!(
        parse_line(RATE),
        vec![EngineEvent::Raw { line: RATE.into() }]
    );
    assert_eq!(
        parse_line("not json"),
        vec![EngineEvent::Raw {
            line: "not json".into()
        }]
    );
    assert!(parse_line("   ").is_empty());
}

#[test]
fn long_tool_output_is_truncated() {
    let big = "x".repeat(70_000);
    let line = json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t","content": big}]}}).to_string();
    match &parse_line(&line)[0] {
        EngineEvent::ToolResult { output, .. } => {
            assert!(output.len() < 66_000);
            assert!(output.ends_with("[truncated]"));
        }
        other => panic!("{other:?}"),
    }
}

fn ctx(session: Option<&str>) -> RunContext {
    let mut dot = fake_dot("cli", Path::new("C:/repo"), WorkspaceMode::Folder);
    dot.spec.model = "haiku".into();
    dot.spec.max_turns = 12;
    dot.spec.policy = Policy::preset(Preset::ReadOnly);
    dot.spec.mcp_servers = Some(json!({"sf": {"type": "stdio", "command": "sf-mcp"}}));
    RunContext {
        run_id: "RUN1".into(),
        dot,
        workspace: "C:/repo".into(),
        prompt: "p".into(),
        session_id: session.map(String::from),
        mcp_url: "http://127.0.0.1:47321/mcp".into(),
        mcp_secret: "SECRET".into(),
        cancel: CancellationToken::new(),
    }
}

#[test]
fn argv_contains_required_flags_in_order() {
    let args = build_args(&ctx(None), Path::new("C:/data/mcp-RUN1.json"));
    let joined = args.join(" ");
    assert!(
        joined.starts_with(
            "-p --output-format stream-json --verbose --model haiku --max-turns 12 \
         --permission-prompt-tool mcp__dots__approve --mcp-config C:/data/mcp-RUN1.json \
         --strict-mcp-config --setting-sources project --allowedTools Read LS Glob Grep \
         --disallowedTools Write Edit NotebookEdit Bash PowerShell"
        ),
        "{joined}"
    );
    assert!(!args.contains(&"--resume".to_string()));
    assert!(
        !args.contains(&"p".to_string()),
        "prompt must not be in argv"
    );
}

#[test]
fn argv_resume_and_user_settings() {
    let mut c = ctx(Some("sess-9"));
    c.dot.spec.use_user_settings = true;
    let args = build_args(&c, Path::new("m.json"));
    assert!(!args.contains(&"--setting-sources".to_string()));
    assert_eq!(&args[args.len() - 2..], ["--resume", "sess-9"]);
}

#[test]
fn mcp_config_merges_dot_servers_and_adds_dots() {
    let cfg = mcp_config(&ctx(None));
    assert_eq!(cfg["mcpServers"]["sf"]["command"], "sf-mcp");
    assert_eq!(cfg["mcpServers"]["dots"]["type"], "http");
    assert_eq!(
        cfg["mcpServers"]["dots"]["url"],
        "http://127.0.0.1:47321/mcp"
    );
    assert_eq!(
        cfg["mcpServers"]["dots"]["headers"]["Authorization"],
        "Bearer SECRET"
    );
}

#[test]
fn resolve_prefers_configured_path() {
    let tmp = tempfile::tempdir().unwrap();
    let exe = tmp.path().join("my-claude.exe");
    std::fs::write(&exe, "").unwrap();
    assert_eq!(resolve_claude(Some(&exe)), Some(exe.clone()));
    assert_eq!(resolve_claude(Some(&tmp.path().join("missing.exe"))), None);
}

#[cfg(windows)]
#[test]
fn resolve_follows_npm_shim_to_native_exe() {
    let tmp = tempfile::tempdir().unwrap();
    let npm = tmp.path().join("npm");
    let native_dir = npm
        .join("node_modules")
        .join("@anthropic-ai")
        .join("claude-code")
        .join("bin");
    std::fs::create_dir_all(&native_dir).unwrap();
    std::fs::write(npm.join("claude"), "#!/bin/sh").unwrap();
    std::fs::write(npm.join("claude.cmd"), "@echo off").unwrap();
    std::fs::write(native_dir.join("claude.exe"), "").unwrap();
    let empty = tmp.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    assert_eq!(
        resolve_in_dirs(vec![empty, npm]),
        Some(native_dir.join("claude.exe"))
    );
}

#[cfg(not(windows))]
#[test]
fn resolve_finds_plain_binary_on_path() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("claude"), "").unwrap();
    assert_eq!(
        resolve_in_dirs(vec![PathBuf::from(tmp.path())]),
        Some(tmp.path().join("claude"))
    );
}
