//! The MCP client against `fake-mcp`, over stdio and Streamable HTTP (MCP/skills spec sections 5
//! and 6).

mod common;

use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

use common::fake_mcp::{tool, FakeMcp};
use dots_core::library::McpServerConfig;
use dots_core::mcp::{
    fingerprint, McpError, McpSession, SessionOptions, CALL_TIMEOUT, PROTOCOL_VERSION,
};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

const WAIT: Duration = Duration::from_secs(10);

fn no_env(_: &str) -> Option<String> {
    None
}

fn names(tools: &[dots_core::mcp::McpTool]) -> Vec<&str> {
    tools.iter().map(|t| t.name.as_str()).collect()
}

async fn open(fake: &FakeMcp) -> McpSession {
    McpSession::open("fake", &fake.stdio(), &no_env, SessionOptions::default())
        .await
        .unwrap()
}

async fn call(session: &McpSession, tool: &str, args: Value) -> Result<Value, McpError> {
    session
        .call_tool(tool, args, CALL_TIMEOUT, &CancellationToken::new())
        .await
}

fn same_dir(a: &str, b: &Path) -> bool {
    std::fs::canonicalize(a).unwrap() == std::fs::canonicalize(b).unwrap()
}

#[tokio::test]
async fn a_stdio_session_initializes_lists_every_page_and_calls() {
    let fake = FakeMcp::new(json!({
        "page_size": 2,
        "tools": [tool("a", true), tool("b", false), tool("c", false)]
    }));
    let work = tempfile::tempdir().unwrap();
    let env = |k: &str| match k {
        "FAKE_TOKEN" => Some("s3cret".to_string()),
        "FAKE_ARG" => Some("on".to_string()),
        _ => None,
    };
    let config = fake.stdio_with(
        vec!["--flag=${FAKE_ARG}".into()],
        BTreeMap::from([("FAKE_MCP_TOKEN".to_string(), "${FAKE_TOKEN}".to_string())]),
    );
    let options = SessionOptions {
        cwd: Some(work.path().to_path_buf()),
        ..SessionOptions::default()
    };
    let session = McpSession::open("fake", &config, &env, options)
        .await
        .unwrap();
    assert_eq!(session.name(), "fake");
    assert_eq!(session.protocol_version(), PROTOCOL_VERSION);
    assert_eq!(session.server_info()["name"], "fake-mcp");

    let tools = session.list_tools().await.unwrap();
    assert_eq!(names(&tools), ["a", "b", "c"]);
    assert!(tools[0].read_only() && !tools[1].read_only());

    let result = call(&session, "b", json!({"q": "x"})).await.unwrap();
    assert_eq!(result["content"][0]["text"], r#"b: {"q":"x"}"#);
    assert_eq!(result.get("isError"), None);

    // The server was started with the variables expanded, in the default folder.
    let start = fake.log()[0]["start"].clone();
    assert_eq!(start["env"]["FAKE_MCP_TOKEN"], "s3cret");
    assert_eq!(start["args"][1], "--flag=on");
    assert!(same_dir(start["cwd"].as_str().unwrap(), work.path()));

    let init = &fake.received("initialize")[0];
    assert_eq!(init["params"]["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(init["params"]["clientInfo"]["name"], "dots");
    let methods: Vec<Value> = fake
        .log()
        .iter()
        .filter_map(|r| r["msg"].get("method").cloned())
        .collect();
    assert_eq!(
        methods,
        json!([
            "initialize",
            "notifications/initialized",
            "tools/list",
            "tools/list",
            "tools/call"
        ])
        .as_array()
        .unwrap()
        .clone()
    );
    let lists = fake.received("tools/list");
    assert_eq!(lists[1]["params"]["cursor"], "2");
    session.close().await;
}

/// A run shares its sessions between tasks: the engine's calls and the gateway's.
#[tokio::test]
async fn a_session_is_shared_between_tasks_and_answers_calls_in_any_order() {
    let mut slow = tool("slow", false);
    slow["delay_ms"] = json!(500);
    let fake = FakeMcp::new(json!({"tools": [slow, tool("fast", true)]}));
    let config = fake.stdio();
    let session = tokio::spawn(async move {
        McpSession::open("fake", &config, &no_env, SessionOptions::default()).await
    })
    .await
    .unwrap()
    .unwrap();
    let session = std::sync::Arc::new(session);
    let first = {
        let session = session.clone();
        tokio::spawn(async move { call(&session, "slow", json!({"n": 1})).await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let second = call(&session, "fast", json!({"n": 2})).await.unwrap();
    assert!(
        !first.is_finished(),
        "the fast call did not wait for the slow one"
    );
    assert_eq!(second["content"][0]["text"], r#"fast: {"n":2}"#);
    let first = first.await.unwrap().unwrap();
    assert_eq!(first["content"][0]["text"], r#"slow: {"n":1}"#);
    session.close().await;
}

#[tokio::test]
async fn the_configs_own_folder_wins_over_the_default() {
    let fake = FakeMcp::new(json!({"tools": []}));
    let own = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let McpServerConfig::Stdio {
        command, args, env, ..
    } = fake.stdio()
    else {
        unreachable!()
    };
    let config = McpServerConfig::Stdio {
        command,
        args,
        env,
        cwd: Some(own.path().to_string_lossy().to_string()),
    };
    let options = SessionOptions {
        cwd: Some(other.path().to_path_buf()),
        ..SessionOptions::default()
    };
    let session = McpSession::open("fake", &config, &no_env, options)
        .await
        .unwrap();
    let start = fake.log()[0]["start"].clone();
    assert!(same_dir(start["cwd"].as_str().unwrap(), own.path()));
    session.close().await;
}

#[tokio::test]
async fn a_missing_variable_or_command_stops_the_server_from_starting() {
    let fake = FakeMcp::new(json!({"tools": []}));
    let config = fake.stdio_with(
        Vec::new(),
        BTreeMap::from([("FAKE_MCP_TOKEN".to_string(), "${FAKE_TOKEN}".to_string())]),
    );
    let err = McpSession::open("fake", &config, &no_env, SessionOptions::default())
        .await
        .err()
        .unwrap();
    assert_eq!(err.to_string(), "could not start: FAKE_TOKEN is not set");
    assert!(fake.log().is_empty(), "nothing was started");

    let missing = McpServerConfig::Stdio {
        command: "dots-no-such-command-xyz".into(),
        args: Vec::new(),
        cwd: None,
        env: BTreeMap::new(),
    };
    let err = McpSession::open("fake", &missing, &no_env, SessionOptions::default())
        .await
        .err()
        .unwrap();
    assert!(
        matches!(&err, McpError::Start(m) if m.contains("dots-no-such-command-xyz")),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_server_that_exits_or_is_silent_at_initialize_cannot_start() {
    let fake = FakeMcp::new(json!({"tools": [], "exit_at_initialize": true}));
    let err = McpSession::open("fake", &fake.stdio(), &no_env, SessionOptions::default())
        .await
        .err()
        .unwrap();
    assert!(matches!(&err, McpError::Start(_)), "{err:?}");

    let fake = FakeMcp::new(json!({"tools": [], "init_delay_ms": 10_000}));
    let options = SessionOptions {
        answer_timeout: Duration::from_millis(300),
        ..SessionOptions::default()
    };
    let begun = Instant::now();
    let err = McpSession::open("fake", &fake.stdio(), &no_env, options)
        .await
        .err()
        .unwrap();
    assert!(matches!(err, McpError::Timeout), "{err:?}");
    assert_eq!(err.to_string(), "did not answer");
    assert!(begun.elapsed() < Duration::from_secs(5));
    assert!(
        fake.gone_within(WAIT).await,
        "a server that did not start is stopped"
    );
}

#[tokio::test]
async fn the_protocol_version_must_be_one_dots_knows() {
    let fake = FakeMcp::new(json!({"tools": [], "protocol_version": "2025-03-26"}));
    let session = open(&fake).await;
    assert_eq!(session.protocol_version(), "2025-03-26");
    session.close().await;

    let fake = FakeMcp::new(json!({"tools": [], "protocol_version": "1999-01-01"}));
    let err = McpSession::open("fake", &fake.stdio(), &no_env, SessionOptions::default())
        .await
        .err()
        .unwrap();
    assert!(
        matches!(&err, McpError::Protocol(m) if m.contains("1999-01-01")),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_tool_error_and_an_error_result_come_back() {
    let mut failing = tool("fail", false);
    failing["error"] = json!({"code": -32000, "message": "it broke"});
    let mut is_error = tool("soft", false);
    is_error["result"] = json!({"content": [{"type": "text", "text": "no"}], "isError": true});
    let fake = FakeMcp::new(json!({"tools": [failing, is_error]}));
    let session = open(&fake).await;

    let err = call(&session, "fail", json!({})).await.unwrap_err();
    assert!(
        matches!(&err, McpError::Server { code: -32000, message } if message == "it broke"),
        "{err:?}"
    );
    let err = call(&session, "nope", json!({})).await.unwrap_err();
    assert!(
        matches!(err, McpError::Server { code: -32602, .. }),
        "{err:?}"
    );
    let result = call(&session, "soft", json!({})).await.unwrap();
    assert_eq!(result["isError"], true);
    assert_eq!(result["content"][0]["text"], "no");
    session.close().await;
}

#[tokio::test]
async fn a_slow_call_times_out_or_is_cancelled_and_the_server_is_told() {
    let mut slow = tool("slow", false);
    slow["delay_ms"] = json!(10_000);
    let fake = FakeMcp::new(json!({"tools": [slow, tool("fast", true)]}));
    let session = open(&fake).await;

    let begun = Instant::now();
    let err = session
        .call_tool(
            "slow",
            json!({}),
            Duration::from_millis(300),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, McpError::Timeout), "{err:?}");
    assert!(begun.elapsed() < Duration::from_secs(5));

    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        trigger.cancel();
    });
    let begun = Instant::now();
    let err = session
        .call_tool("slow", json!({}), CALL_TIMEOUT, &cancel)
        .await
        .unwrap_err();
    assert!(matches!(err, McpError::Cancelled), "{err:?}");
    assert!(begun.elapsed() < Duration::from_secs(5));

    // The server hears of both, and the session goes on.
    assert!(
        fake.wait_for(WAIT, |_| fake_cancelled_count(&fake) == 2)
            .await
    );
    let calls = fake.received("tools/call");
    let cancelled = fake.received("notifications/cancelled");
    assert_eq!(cancelled[0]["params"]["requestId"], calls[0]["id"]);
    assert_eq!(cancelled[1]["params"]["requestId"], calls[1]["id"]);
    assert!(call(&session, "fast", json!({})).await.is_ok());
    session.close().await;
}

fn fake_cancelled_count(fake: &FakeMcp) -> usize {
    fake.received("notifications/cancelled").len()
}

#[tokio::test]
async fn a_list_change_is_surfaced_and_the_new_list_read() {
    let mut switch = tool("switch", false);
    switch["switch"] = json!(true);
    let fake = FakeMcp::new(json!({
        "tools": [tool("a", true), switch],
        "after_switch": [tool("a", false), tool("z", true)]
    }));
    let session = open(&fake).await;
    let mut changed = session.list_changed();
    let before = session.list_tools().await.unwrap();
    assert_eq!(*changed.borrow(), 0);

    call(&session, "switch", json!({})).await.unwrap();
    tokio::time::timeout(WAIT, changed.changed())
        .await
        .expect("no list change seen")
        .unwrap();
    assert_eq!(*changed.borrow_and_update(), 1);
    let after = session.list_tools().await.unwrap();
    assert_eq!(names(&after), ["a", "z"]);
    assert_ne!(fingerprint(&before), fingerprint(&after));
    session.close().await;
}

#[tokio::test]
async fn requests_from_the_server_are_refused_except_ping() {
    let mut roots = tool("roots", false);
    roots["ask_client"] = json!("roots/list");
    let mut ping = tool("ping", false);
    ping["ask_client"] = json!("ping");
    let fake = FakeMcp::new(json!({"tools": [roots, ping]}));
    let session = open(&fake).await;
    call(&session, "roots", json!({})).await.unwrap();
    call(&session, "ping", json!({})).await.unwrap();
    // `fake-mcp` gives its requests the id `s-<method>`; the answers are in its log.
    let answer = |id: &str| {
        fake.log()
            .into_iter()
            .find(|r| r["msg"]["id"] == id && r["msg"].get("method").is_none())
            .map(|r| r["msg"].clone())
    };
    assert!(fake.wait_for(WAIT, |r| r["msg"]["id"] == "s-ping").await);
    assert!(
        fake.wait_for(WAIT, |r| r["msg"]["id"] == "s-roots/list")
            .await
    );
    assert_eq!(answer("s-roots/list").unwrap()["error"]["code"], -32601);
    assert_eq!(answer("s-ping").unwrap()["result"], json!({}));
    session.close().await;
}

#[tokio::test]
async fn a_server_that_dies_stops_its_session() {
    let mut crash = tool("crash", false);
    crash["exit"] = json!(true);
    let fake = FakeMcp::new(json!({"tools": [crash, tool("a", true)]}));
    let session = open(&fake).await;
    let err = call(&session, "crash", json!({})).await.unwrap_err();
    assert!(matches!(err, McpError::Stopped), "{err:?}");
    assert_eq!(err.to_string(), "the server stopped");
    assert!(session.is_stopped());
    let begun = Instant::now();
    let err = call(&session, "a", json!({})).await.unwrap_err();
    assert!(matches!(err, McpError::Stopped), "{err:?}");
    assert!(
        begun.elapsed() < Duration::from_secs(1),
        "no wait, no restart"
    );
    assert!(matches!(session.list_tools().await, Err(McpError::Stopped)));
    session.close().await;
}

#[tokio::test]
async fn close_stops_the_server_and_so_does_dropping_the_session() {
    let fake = FakeMcp::new(json!({"tools": []}));
    let session = open(&fake).await;
    assert!(fake.alive());
    session.close().await;
    assert!(!fake.alive(), "close waits until the server is gone");
    assert!(session.is_stopped());
    session.close().await; // twice is fine

    // A server that does not end when its input closes is killed after the grace time.
    let fake = FakeMcp::new(json!({"tools": [], "ignore_eof": true}));
    let options = SessionOptions {
        close_grace: Duration::from_millis(200),
        ..SessionOptions::default()
    };
    let session = McpSession::open("fake", &fake.stdio(), &no_env, options)
        .await
        .unwrap();
    let begun = Instant::now();
    session.close().await;
    assert!(!fake.alive());
    assert!(begun.elapsed() < Duration::from_secs(5));

    let fake = FakeMcp::new(json!({"tools": [], "ignore_eof": true}));
    let session = open(&fake).await;
    drop(session);
    assert!(
        fake.gone_within(WAIT).await,
        "dropping the session stops it"
    );
}

#[tokio::test]
async fn a_server_that_writes_much_to_stderr_is_not_blocked() {
    let fake = FakeMcp::new(json!({"tools": [tool("a", true)], "stderr_bytes": 1_000_000}));
    let options = SessionOptions {
        answer_timeout: Duration::from_secs(20),
        ..SessionOptions::default()
    };
    let session = McpSession::open("fake", &fake.stdio(), &no_env, options)
        .await
        .unwrap();
    assert_eq!(names(&session.list_tools().await.unwrap()), ["a"]);
    session.close().await;
}

// --- Streamable HTTP ---

fn bearer(var: &str) -> BTreeMap<String, String> {
    BTreeMap::from([("Authorization".to_string(), format!("Bearer ${{{var}}}"))])
}

fn token_env(k: &str) -> Option<String> {
    (k == "FAKE_TOKEN").then(|| "s3cret".to_string())
}

#[tokio::test]
async fn an_http_session_with_json_answers_keeps_its_session_id() {
    let fake = FakeMcp::new(json!({
        "page_size": 1,
        "tools": [tool("a", true), tool("b", false)],
        "require_header": {"authorization": "Bearer s3cret"}
    }));
    let http = fake.serve_http().await;
    let session = McpSession::open(
        "web",
        &http.config(bearer("FAKE_TOKEN")),
        &token_env,
        SessionOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(session.protocol_version(), PROTOCOL_VERSION);
    assert_eq!(names(&session.list_tools().await.unwrap()), ["a", "b"]);
    let result = call(&session, "a", json!({"q": 1})).await.unwrap();
    assert_eq!(result["content"][0]["text"], r#"a: {"q":1}"#);
    session.close().await;

    let log = fake.log();
    let posts: Vec<&Value> = log.iter().filter(|r| r["http"] == "POST").collect();
    assert_eq!(posts[0]["msg"]["method"], "initialize");
    assert_eq!(posts[0]["session"], Value::Null);
    let id = posts[1]["session"]
        .as_str()
        .expect("a session id")
        .to_string();
    for p in &posts[1..] {
        assert_eq!(p["session"], id.as_str(), "{p}");
        assert_eq!(p["protocol"], PROTOCOL_VERSION, "{p}");
    }
    assert!(posts.iter().all(|p| p["auth"] == "Bearer s3cret"));
    let delete = log.iter().find(|r| r["http"] == "DELETE").expect("DELETE");
    assert_eq!(delete["session"], id.as_str());

    // A wrong header is refused, and the error does not show it.
    let err = McpSession::open(
        "web",
        &http.config(bearer("FAKE_TOKEN")),
        &|_: &str| Some("wrong".to_string()),
        SessionOptions::default(),
    )
    .await
    .err()
    .unwrap();
    assert!(err.to_string().contains("401"), "{err}");
    assert!(!err.to_string().contains("wrong"), "{err}");
}

#[tokio::test]
async fn an_http_session_reads_event_streams_with_a_list_change_in_them() {
    let mut switch = tool("switch", false);
    switch["switch"] = json!(true);
    let fake = FakeMcp::new(json!({
        "sse": true,
        "tools": [tool("a", true), switch],
        "after_switch": [tool("b", false)]
    }));
    let http = fake.serve_http().await;
    let session = McpSession::open(
        "web",
        &http.config(BTreeMap::new()),
        &no_env,
        SessionOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(names(&session.list_tools().await.unwrap()).len(), 2);
    let changed = session.list_changed();
    call(&session, "switch", json!({})).await.unwrap();
    assert_eq!(
        *changed.borrow(),
        1,
        "the notification came before the answer"
    );
    assert_eq!(names(&session.list_tools().await.unwrap()), ["b"]);

    // A request in the stream is answered with a POST of its own.
    let fake2 = FakeMcp::new(json!({"sse": true, "tools": [roots_tool()]}));
    let http2 = fake2.serve_http().await;
    let session2 = McpSession::open(
        "web2",
        &http2.config(BTreeMap::new()),
        &no_env,
        SessionOptions::default(),
    )
    .await
    .unwrap();
    call(&session2, "roots", json!({})).await.unwrap();
    assert!(
        fake2
            .wait_for(WAIT, |r| r["msg"]["id"] == "s-roots/list"
                && r["msg"]["error"]["code"] == -32601)
            .await
    );
    session.close().await;
    session2.close().await;
}

fn roots_tool() -> Value {
    let mut roots = tool("roots", false);
    roots["ask_client"] = json!("roots/list");
    roots
}

#[tokio::test]
async fn an_http_list_change_on_the_get_stream_is_surfaced() {
    let mut switch = tool("switch", false);
    switch["switch"] = json!(true);
    let fake = FakeMcp::new(json!({
        "get_stream": true,
        "tools": [switch],
        "after_switch": [tool("b", false)]
    }));
    let http = fake.serve_http().await;
    let session = McpSession::open(
        "web",
        &http.config(BTreeMap::new()),
        &no_env,
        SessionOptions::default(),
    )
    .await
    .unwrap();
    let mut changed = session.list_changed();
    assert!(
        fake.wait_for(WAIT, |r| r["http"] == "GET").await,
        "the client listens on the GET stream"
    );
    call(&session, "switch", json!({})).await.unwrap();
    tokio::time::timeout(WAIT, changed.changed())
        .await
        .expect("no list change seen")
        .unwrap();
    assert_eq!(names(&session.list_tools().await.unwrap()), ["b"]);
    session.close().await;
}

#[tokio::test]
async fn an_http_call_can_be_cancelled_and_a_dead_server_stops_the_session() {
    let mut slow = tool("slow", false);
    slow["delay_ms"] = json!(10_000);
    let mut crash = tool("crash", false);
    crash["exit"] = json!(true);
    let fake = FakeMcp::new(json!({"tools": [slow, crash, tool("a", true)]}));
    let mut http = fake.serve_http().await;
    let session = McpSession::open(
        "web",
        &http.config(BTreeMap::new()),
        &no_env,
        SessionOptions::default(),
    )
    .await
    .unwrap();

    let cancel = CancellationToken::new();
    cancel.cancel();
    let err = session
        .call_tool("slow", json!({}), CALL_TIMEOUT, &cancel)
        .await
        .unwrap_err();
    assert!(matches!(err, McpError::Cancelled), "{err:?}");
    assert!(
        fake.wait_for(WAIT, |r| r["msg"]["method"] == "notifications/cancelled")
            .await
    );

    // The call that kills it fails; once the server is gone, the session is stopped.
    assert!(call(&session, "crash", json!({})).await.is_err());
    let _ = http.child.wait().await;
    let err = call(&session, "a", json!({})).await.unwrap_err();
    assert!(matches!(err, McpError::Stopped), "{err:?}");
    assert!(session.is_stopped());
    session.close().await;
}

#[tokio::test]
async fn an_http_server_that_is_not_there_cannot_start() {
    // A port that was just free.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let config = McpServerConfig::Http {
        url: format!("http://127.0.0.1:{port}/mcp?key=${{FAKE_TOKEN}}"),
        headers: BTreeMap::new(),
    };
    let err = McpSession::open("web", &config, &token_env, SessionOptions::default())
        .await
        .err()
        .unwrap();
    assert!(matches!(&err, McpError::Start(_)), "{err:?}");
    assert!(
        !err.to_string().contains("s3cret"),
        "the URL is not shown: {err}"
    );
}

// --- Limits and redirects ---

/// A little more than the client accepts as one message.
const TOO_BIG: u64 = 17 * 1024 * 1024;

fn big_tool() -> Value {
    let mut big = tool("big", false);
    big["big_bytes"] = json!(TOO_BIG);
    big
}

#[tokio::test]
async fn an_http_cancel_does_not_wait_for_a_server_that_stalls() {
    let mut slow = tool("slow", false);
    slow["delay_ms"] = json!(10_000);
    let fake = FakeMcp::new(json!({"tools": [slow], "stall_cancelled_ms": 8_000}));
    let http = fake.serve_http().await;
    let session = McpSession::open(
        "web",
        &http.config(BTreeMap::new()),
        &no_env,
        SessionOptions::default(),
    )
    .await
    .unwrap();

    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        trigger.cancel();
    });
    let begun = Instant::now();
    let err = session
        .call_tool("slow", json!({}), CALL_TIMEOUT, &cancel)
        .await
        .unwrap_err();
    assert!(matches!(err, McpError::Cancelled), "{err:?}");
    assert!(
        begun.elapsed() < Duration::from_secs(1),
        "{:?}",
        begun.elapsed()
    );

    // A timeout is as quick, and the notice still reaches the server.
    let begun = Instant::now();
    let err = session
        .call_tool(
            "slow",
            json!({}),
            Duration::from_millis(100),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, McpError::Timeout), "{err:?}");
    assert!(
        begun.elapsed() < Duration::from_secs(1),
        "{:?}",
        begun.elapsed()
    );
    assert!(
        fake.wait_for(WAIT, |r| r["msg"]["method"] == "notifications/cancelled")
            .await
    );
}

#[tokio::test]
async fn a_stdio_line_past_the_limit_stops_the_server_instead_of_filling_memory() {
    let fake = FakeMcp::new(json!({"tools": [big_tool(), tool("a", true)]}));
    let session = open(&fake).await;
    let err = call(&session, "big", json!({})).await.unwrap_err();
    assert!(matches!(err, McpError::Stopped), "{err:?}");
    assert!(session.is_stopped());
    assert!(
        fake.gone_within(WAIT).await,
        "the server that wrote it is shut down"
    );
    session.close().await;
}

#[tokio::test]
async fn an_http_body_or_event_past_the_limit_fails_that_request_only() {
    for sse in [false, true] {
        let fake = FakeMcp::new(json!({"sse": sse, "tools": [big_tool(), tool("a", true)]}));
        let http = fake.serve_http().await;
        let session = McpSession::open(
            "web",
            &http.config(BTreeMap::new()),
            &no_env,
            SessionOptions::default(),
        )
        .await
        .unwrap();
        let err = call(&session, "big", json!({})).await.unwrap_err();
        assert!(
            matches!(&err, McpError::Protocol(m) if m.contains("too large")),
            "sse {sse}: {err:?}"
        );
        assert!(call(&session, "a", json!({})).await.is_ok(), "sse {sse}");
        session.close().await;
    }
}

#[tokio::test]
async fn an_http_redirect_is_an_error_and_is_not_followed() {
    let target = FakeMcp::new(json!({"tools": []}));
    let target_http = target.serve_http().await;
    let fake = FakeMcp::new(json!({"tools": [], "redirect_to": target_http.url}));
    let http = fake.serve_http().await;
    let headers = BTreeMap::from([("X-Api-Key".to_string(), "${FAKE_TOKEN}".to_string())]);
    let err = McpSession::open(
        "web",
        &http.config(headers),
        &token_env,
        SessionOptions::default(),
    )
    .await
    .err()
    .unwrap();
    assert!(
        matches!(&err, McpError::Protocol(m) if m.starts_with("the server answered HTTP 302")),
        "{err:?}"
    );
    assert!(!err.to_string().contains("s3cret"), "{err}");
    assert!(
        target.log().iter().all(|r| r.get("http").is_none()),
        "the target was never contacted: {:?}",
        target.log()
    );
}
