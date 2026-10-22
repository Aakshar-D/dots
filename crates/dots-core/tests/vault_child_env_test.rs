//! What a child process starts with (env vault spec §8): the base of Dots' environment, never
//! the rest, then what it is given.

mod common;

use std::collections::BTreeMap;

use common::fake_mcp::{tool, FakeMcp};
use dots_core::mcp::{McpSession, SessionOptions};
use dots_core::vault::child_env::{base_env, Child, BASE_ENV, CLI_ENV, CLI_PREFIXES};
use serde_json::json;

fn names(child: Child) -> Vec<String> {
    base_env(child)
        .into_iter()
        .map(|(k, _)| k.to_string_lossy().to_ascii_uppercase())
        .collect()
}

#[test]
fn the_base_keeps_what_a_program_needs_and_drops_the_rest() {
    std::env::set_var("DOTS_TEST_BASE_LEAK", "from-dots");
    std::env::set_var("ANTHROPIC_DOTS_TEST_BASE", "for-the-cli");
    std::env::set_var("https_proxy", "http://proxy.example:8080");
    let other = names(Child::Other);
    assert!(other.contains(&"PATH".to_string()));
    assert!(other.contains(&"HTTPS_PROXY".to_string()));
    assert!(!other.contains(&"DOTS_TEST_BASE_LEAK".to_string()));
    assert!(!other.contains(&"ANTHROPIC_DOTS_TEST_BASE".to_string()));
    let cli = names(Child::Cli);
    assert!(cli.contains(&"ANTHROPIC_DOTS_TEST_BASE".to_string()));
    assert!(!cli.contains(&"DOTS_TEST_BASE_LEAK".to_string()));
    // The list itself: what the spec names, and nothing that holds a secret by its name.
    for name in [
        "SystemRoot",
        "PATH",
        "TEMP",
        "USERPROFILE",
        "APPDATA",
        "NO_PROXY",
    ] {
        assert!(BASE_ENV.contains(&name), "{name}");
    }
    assert!(!BASE_ENV.iter().any(|n| dots_core::library::looks_secret(n)));
}

#[tokio::test]
async fn an_mcp_server_gets_its_own_env_but_not_dots_environment() {
    std::env::set_var("FAKE_MCP_DOTS_TEST_LEAK", "from-dots");
    let fake = FakeMcp::new(json!({"tools": [tool("read", true)]}));
    let config = fake.stdio_with(
        Vec::new(),
        BTreeMap::from([("FAKE_MCP_TOKEN".to_string(), "given".to_string())]),
    );
    let session = McpSession::open("srv", &config, &|_| None, SessionOptions::default())
        .await
        .unwrap();
    session.close().await;
    let start = fake
        .log()
        .into_iter()
        .find_map(|r| r.get("start").cloned())
        .unwrap();
    assert_eq!(start["env"], json!({"FAKE_MCP_TOKEN": "given"}));
}

#[test]
fn the_lists_are_pinned() {
    assert_eq!(
        BASE_ENV,
        [
            "SystemRoot",
            "SystemDrive",
            "windir",
            "ComSpec",
            "PATH",
            "PATHEXT",
            "PSModulePath",
            "TEMP",
            "TMP",
            "HOME",
            "USERPROFILE",
            "HOMEDRIVE",
            "HOMEPATH",
            "APPDATA",
            "LOCALAPPDATA",
            "ProgramData",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramW6432",
            "CommonProgramFiles",
            "CommonProgramFiles(x86)",
            "CommonProgramW6432",
            "NUMBER_OF_PROCESSORS",
            "PROCESSOR_ARCHITECTURE",
            "OS",
            "USERNAME",
            "USERDOMAIN",
            "COMPUTERNAME",
            "LANG",
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "NO_PROXY",
            "NODE_EXTRA_CA_CERTS",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "REQUESTS_CA_BUNDLE",
            "CURL_CA_BUNDLE",
            "ALL_PROXY",
            "ALLUSERSPROFILE",
            "PUBLIC",
        ]
    );
    assert_eq!(
        CLI_ENV,
        [
            "CLAUDE_CONFIG_DIR",
            "DISABLE_TELEMETRY",
            "DISABLE_AUTOUPDATER",
            "DISABLE_ERROR_REPORTING",
            "MCP_TIMEOUT",
            "BASH_DEFAULT_TIMEOUT_MS",
            "BASH_MAX_TIMEOUT_MS",
            "MAX_MCP_OUTPUT_TOKENS",
        ]
    );
    assert_eq!(CLI_PREFIXES, ["ANTHROPIC_", "CLAUDE_CODE_"]);
}
