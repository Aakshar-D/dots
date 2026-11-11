mod common;

use std::path::{Path, PathBuf};

use common::fake_dot;
use dots_core::engine::claude::args::{
    build_args, cli_settings, gateway_entry, mac_claude_dirs, mcp_config, resolve_claude,
    resolve_in_dirs, settings_path,
};
use dots_core::engine::claude::parse::parse_line;
use dots_core::engine::{EngineEvent, RunContext};
use dots_core::model::WorkspaceMode;
use dots_core::policy::{Action, Policy, Preset, Rule, PLUMBING_TOOLS};
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
        vec![
            EngineEvent::AssistantText {
                text: "pong".into()
            },
            // The turn's fill (context rules spec section 3).
            EngineEvent::Context {
                fill: Some(2),
                window: None
            }
        ]
    );
    assert_eq!(
        parse_line(TOOL_USE),
        vec![EngineEvent::ToolCall {
            id: "toolu_1".into(),
            tool: "Bash".into(),
            input: json!({"command": "git status", "description": "Check"}),
            parent: None,
        }]
    );
}

#[test]
fn a_helpers_call_names_the_call_that_started_it() {
    // A `Task` helper's TodoWrite (run plan spec section 3): its line carries the helper's call.
    let helper = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"toolu_2","name":"TodoWrite","input":{"todos":[{"content":"Read the logs","status":"in_progress","activeForm":"Reading the logs"}]}}]},"parent_tool_use_id":"toolu_task"}"#;
    assert_eq!(
        parse_line(helper),
        vec![EngineEvent::ToolCall {
            id: "toolu_2".into(),
            tool: "TodoWrite".into(),
            input: json!({"todos": [{"content": "Read the logs", "status": "in_progress", "activeForm": "Reading the logs"}]}),
            parent: Some("toolu_task".into()),
        }]
    );
    // The dot's own call: `null`, or no field, names no helper, and the stored event has no key.
    let own = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t","name":"Read","input":{}}]},"parent_tool_use_id":null}"#;
    let events = parse_line(own);
    assert!(matches!(
        &events[..],
        [EngineEvent::ToolCall { parent: None, .. }]
    ));
    let stored = serde_json::to_value(&events[0]).unwrap();
    assert!(stored.get("parent").is_none(), "{stored}");
    // An event stored before the field existed still reads.
    let old: EngineEvent = serde_json::from_value(
        json!({"kind": "tool_call", "id": "t", "tool": "Read", "input": {}}),
    )
    .unwrap();
    assert!(matches!(old, EngineEvent::ToolCall { parent: None, .. }));
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
                tokens_out: 4,
                cost_usd: None,
                cache_read: 10,
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
                tokens_out: 1,
                cost_usd: None,
                cache_read: 0,
            },
            EngineEvent::Failed {
                error: "error_max_turns".into()
            },
        ]
    );
}

#[test]
fn unknown_and_invalid_lines_are_raw_and_blank_is_nothing() {
    // A rate limit line is read, not kept as raw text (usage and limits spec section 3).
    assert_eq!(
        parse_line(RATE),
        vec![EngineEvent::Limits {
            status: "allowed".into(),
            windows: vec![],
            resets_at: None,
        }]
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
    RunContext {
        run_id: "RUN1".into(),
        dot,
        workspace: "C:/repo".into(),
        prompt: "p".into(),
        session_id: session.map(String::from),
        mcp_url: "http://127.0.0.1:47321/mcp".into(),
        mcp_secret: "SECRET".into(),
        cancel: CancellationToken::new(),
        approved_calls: Vec::new(),
        tools: Default::default(),
        autocompact_pct: None,
        guardrails: None,
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
         --strict-mcp-config --setting-sources  --allowedTools Read Skill \
         mcp__dots__notify mcp__dots__ask mcp__dots__follow_up mcp__dots__remember \
         mcp__dots__forget mcp__dots__recall mcp__dots__propose_app mcp__dots__copy_app \
         TodoWrite Task Agent ExitPlanMode EnterPlanMode ToolSearch ListMcpResourcesTool \
         BashOutput KillShell KillBash \
         --disallowedTools Write Edit NotebookEdit Bash PowerShell \
         --settings C:/data/mcp-RUN1.settings.json"
        ),
        "{joined}"
    );
    assert!(!args.contains(&"--resume".to_string()));
    // Empty setting sources: no user, project or local settings reach headless runs.
    assert!(args.windows(2).any(|w| w == ["--setting-sources", ""]));
    assert!(
        !args.contains(&"p".to_string()),
        "prompt must not be in argv"
    );
}

#[test]
fn argv_asks_for_private_folders_and_never_for_plumbing() {
    let mut c = ctx(None);
    c.dot.spec.policy = Policy::preset(Preset::Trusted);
    // A prefix deny and an ask rule keep their tools out of the allow list.
    c.dot
        .spec
        .policy
        .rules
        .push(Rule::new("Todo*", Action::Deny));
    c.dot
        .spec
        .policy
        .rules
        .push(Rule::new("Agent", Action::Ask));
    let args = build_args(&c, Path::new("m.json"));
    let allowed: Vec<&String> = args
        .iter()
        .skip_while(|a| *a != "--allowedTools")
        .skip(1)
        .take_while(|a| !a.starts_with("--"))
        .collect();
    for tool in PLUMBING_TOOLS
        .iter()
        .filter(|t| !["TodoWrite", "Agent"].contains(*t))
    {
        assert!(allowed.iter().any(|a| a == tool), "{tool} in {allowed:?}");
    }
    // Searches always reach `approve`, so Dots can judge the folder they start in.
    c.dot
        .spec
        .policy
        .rules
        .push(Rule::new("Grep(src/**)", Action::Allow));
    let searched = build_args(&c, Path::new("m.json"));
    let allowed_now: Vec<&String> = searched
        .iter()
        .skip_while(|a| *a != "--allowedTools")
        .skip(1)
        .take_while(|a| !a.starts_with("--"))
        .collect();
    assert!(
        !allowed_now.iter().any(|a| ["Grep", "Glob", "LS"]
            .iter()
            .any(|t| a == t || a.starts_with(&format!("{t}(")))),
        "{allowed_now:?}"
    );
    // A denied or asked plumbing tool is not also allowed.
    assert!(!allowed.iter().any(|a| *a == "TodoWrite" || *a == "Agent"));
    // The settings go in a file next to the MCP config, never inline: a configured `.cmd`
    // program would hand an inline JSON argument to cmd.exe.
    let i = args
        .iter()
        .position(|a| a == "--settings")
        .expect("--settings");
    assert_eq!(
        args[i + 1],
        settings_path(Path::new("m.json")).to_string_lossy()
    );
    let settings = cli_settings();
    let ask: Vec<&str> = settings["permissions"]["ask"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for rule in [
        "Read(//**/.ssh/**)",
        "Edit(//**/.env)",
        "Edit(//**/.aws/**)",
        "Read(//**/.env.*)",
        "Bash(*.ssh*)",
        "PowerShell(*.npmrc*)",
    ] {
        assert!(ask.contains(&rule), "{rule} in {ask:?}");
    }
    // The settings come with user settings too.
    c.dot.spec.use_user_settings = true;
    assert!(build_args(&c, Path::new("m.json")).contains(&"--settings".to_string()));
}

#[test]
fn argv_resume_and_user_settings() {
    let mut c = ctx(Some("sess-9"));
    c.dot.spec.use_user_settings = true;
    let args = build_args(&c, Path::new("m.json"));
    assert!(args
        .windows(2)
        .any(|w| w == ["--setting-sources", "user,project,local"]));
    assert_eq!(&args[args.len() - 2..], ["--resume", "sess-9"]);
}

#[test]
fn mcp_config_lists_dots_and_a_gateway_entry_per_open_server() {
    // No open session: Dots' own server only.
    assert_eq!(
        mcp_config(&ctx(None)),
        json!({"mcpServers": {"dots": {"type": "http", "url": "http://127.0.0.1:47321/mcp",
               "headers": {"Authorization": "Bearer SECRET"}}}})
    );
    // An open session is reached through the gateway with the run's secret, never directly.
    assert_eq!(
        gateway_entry("http://127.0.0.1:47321/mcp", "SECRET", "sf"),
        json!({"type": "http", "url": "http://127.0.0.1:47321/mcp/s/sf",
               "headers": {"Authorization": "Bearer SECRET"}})
    );
}

#[test]
fn init_plugin_errors_become_notices() {
    let line = json!({"type": "system", "subtype": "init", "session_id": "s1", "plugin_errors": [
        {"plugin": "forms", "error": "manifest is not valid JSON"},
        {"message": "no plugin.json"},
        "dots-skills: skills/pdf has no SKILL.md",
        {"code": 7}
    ]})
    .to_string();
    let notice = |text: &str| EngineEvent::Notice { text: text.into() };
    assert_eq!(
        parse_line(&line),
        vec![
            EngineEvent::SessionStarted {
                session_id: "s1".into()
            },
            notice("plugin forms: manifest is not valid JSON"),
            notice("plugin error: no plugin.json"),
            notice("plugin error: dots-skills: skills/pdf has no SKILL.md"),
            notice(r#"plugin error: {"code":7}"#),
        ]
    );
    // An init without errors is the session alone, as before.
    assert_eq!(parse_line(INIT).len(), 1);
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

#[test]
fn macos_also_looks_where_the_cli_installers_put_it() {
    let dirs = mac_claude_dirs(Some(PathBuf::from("/Users/u")));
    assert_eq!(dirs[0], PathBuf::from("/Users/u/.local/bin"));
    assert!(dirs.contains(&PathBuf::from("/Users/u/.claude/local")));
    assert!(dirs.contains(&PathBuf::from("/opt/homebrew/bin")));
    assert!(dirs.contains(&PathBuf::from("/usr/local/bin")));
    assert_eq!(
        mac_claude_dirs(None),
        [
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin")
        ]
    );
}

fn allowed_tools(args: &[String]) -> Vec<String> {
    args.iter()
        .skip_while(|a| *a != "--allowedTools")
        .skip(1)
        .take_while(|a| !a.starts_with("--"))
        .cloned()
        .collect()
}

#[test]
fn with_the_guard_on_every_shell_command_reaches_approve() {
    let mut c = ctx(None);
    c.dot.spec.policy = Policy::preset(Preset::Sandboxed);
    // Sandboxed allows `git status`, `git diff` and `git add`; a wide rule covers `git push`.
    c.dot
        .spec
        .policy
        .rules
        .push(Rule::new("PowerShell(git:*)", Action::Allow));
    let on = allowed_tools(&build_args(&c, Path::new("m.json")));
    assert!(
        !on.iter()
            .any(|a| a.starts_with("Bash(") || a.starts_with("PowerShell(")),
        "{on:?}"
    );
    // The other allow rules stay with the CLI.
    assert!(on.iter().any(|a| a == "Write(./**)"), "{on:?}");
    // Switched off, the CLI gets the shell rules as before.
    c.dot.spec.policy.ask_send_delete = false;
    let off = allowed_tools(&build_args(&c, Path::new("m.json")));
    assert!(off.iter().any(|a| a == "Bash(git status:*)"), "{off:?}");
    assert!(off.iter().any(|a| a == "PowerShell(git:*)"), "{off:?}");
}

fn disallowed_tools(args: &[String]) -> Vec<String> {
    args.iter()
        .skip_while(|a| *a != "--disallowedTools")
        .skip(1)
        .take_while(|a| !a.starts_with("--"))
        .cloned()
        .collect()
}

fn rails(role: dots_core::accounts::GuardRole) -> dots_core::accounts::DotRails {
    dots_core::accounts::DotRails {
        role,
        guardrails: dots_core::accounts::Guardrails::defaults(),
        workspace_name: "Payments".into(),
    }
}

#[test]
fn guardrails_send_every_command_to_approve_and_deny_a_members_protected_calls() {
    use dots_core::accounts::GuardRole;
    use dots_core::engine::claude::args::cli_settings_for;
    let mut c = ctx(None);
    c.dot.spec.policy = Policy::preset(Preset::Sandboxed);
    c.dot.spec.policy.ask_send_delete = false;
    let without = build_args(&c, Path::new("m.json"));
    assert!(allowed_tools(&without).contains(&"Bash(git status:*)".to_string()));
    assert!(!disallowed_tools(&without)
        .iter()
        .any(|d| d.contains("migrations")));

    c.guardrails = Some(rails(GuardRole::Admin));
    let admin = build_args(&c, Path::new("m.json"));
    assert!(!allowed_tools(&admin).iter().any(|a| a.starts_with("Bash(")));
    // Writes reach `approve` too, so a protected path is read by Dots, not the CLI.
    assert!(!allowed_tools(&admin).contains(&"Write(./**)".to_string()));
    // An Admin's protected calls ask: nothing is denied up front.
    assert_eq!(disallowed_tools(&admin), disallowed_tools(&without));
    let ask = cli_settings_for(&c)["permissions"]["ask"].clone();
    let ask: Vec<String> = serde_json::from_value(ask).unwrap();
    for rule in [
        "Write(**/migrations/**)",
        "Edit(.github/**)",
        "NotebookEdit(infra/**)",
    ] {
        assert!(ask.contains(&rule.to_string()), "{rule}");
    }
    // The private folders keep their own.
    assert!(cli_settings()["permissions"]["ask"]
        .as_array()
        .unwrap()
        .iter()
        .all(|r| ask.contains(&r.as_str().unwrap().to_string())));

    // A Member's protected calls are asked of the CLI, not denied by it, so they reach
    // `approve` and the dot reads "This needs an Admin" (guardrails spec §4.2). The dot's own
    // policy denies stay in `--disallowedTools`.
    c.guardrails = Some(rails(GuardRole::Member));
    c.dot
        .spec
        .policy
        .rules
        .push(Rule::new("Bash(curl:*)", Action::Deny));
    let member = disallowed_tools(&build_args(&c, Path::new("m.json")));
    assert!(member.contains(&"Bash(curl:*)".to_string()), "{member:?}");
    let ask: Vec<String> =
        serde_json::from_value(cli_settings_for(&c)["permissions"]["ask"].clone()).unwrap();
    for rule in [
        "Edit(**/migrations/**)",
        "Write(**/Cargo.lock)",
        "Bash(git push origin main:*)",
        "PowerShell(git push --force:*)",
        "Bash(gh pr merge:*)",
        "Bash(rm -rf:*)",
        "Bash(terraform apply:*)",
    ] {
        assert!(!member.contains(&rule.to_string()), "{rule}: {member:?}");
        assert!(ask.contains(&rule.to_string()), "{rule}: {ask:?}");
    }
    // Without guardrails the settings are the private folders' alone.
    c.guardrails = None;
    assert_eq!(cli_settings_for(&c), cli_settings());
}

#[test]
fn a_guarded_dot_sends_every_write_to_approve_and_reads_no_settings() {
    use dots_core::accounts::GuardRole;
    let mut c = ctx(None);
    c.dot.spec.policy = Policy::preset(Preset::Sandboxed);
    c.dot
        .spec
        .policy
        .rules
        .push(Rule::new("NotebookEdit(src/**)", Action::Allow));
    c.dot
        .spec
        .policy
        .rules
        .push(Rule::new("MultiEdit(src/**)", Action::Allow));
    c.dot.spec.use_user_settings = true;
    c.guardrails = Some(rails(GuardRole::Admin));
    let args = build_args(&c, Path::new("m.json"));
    for tool in [
        "Bash",
        "PowerShell",
        "Write",
        "Edit",
        "MultiEdit",
        "NotebookEdit",
    ] {
        assert!(
            !allowed_tools(&args)
                .iter()
                .any(|a| a.split('(').next() == Some(tool)),
            "{tool}: {:?}",
            allowed_tools(&args)
        );
    }
    let at = args.iter().position(|a| a == "--setting-sources").unwrap();
    assert_eq!(args[at + 1], "");
    // Without guardrails the person's settings choice holds.
    c.guardrails = None;
    let args = build_args(&c, Path::new("m.json"));
    let at = args.iter().position(|a| a == "--setting-sources").unwrap();
    assert_eq!(args[at + 1], "user,project,local");
    assert!(allowed_tools(&args).contains(&"Write(./**)".to_string()));
}
