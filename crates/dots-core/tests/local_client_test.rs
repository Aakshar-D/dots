mod llm;

use std::time::Duration;

use dots_core::engine::local::client::{completions_url, parse_completion, ChatClient};
use llm::{call, completion, text, Llm};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use wiremock::ResponseTemplate;

fn client() -> ChatClient {
    ChatClient::new(vec![Duration::ZERO, Duration::ZERO])
}

#[test]
fn completions_url_accepts_v1_and_trailing_slashes() {
    for endpoint in [
        "http://127.0.0.1:1234",
        "http://127.0.0.1:1234/",
        "http://127.0.0.1:1234/v1",
        " http://127.0.0.1:1234/v1/ ",
    ] {
        assert_eq!(
            completions_url(endpoint),
            "http://127.0.0.1:1234/v1/chat/completions"
        );
    }
}

#[test]
fn parse_completion_normalizes_tool_calls() {
    let reply = json!({
        "choices": [{"message": {
            "role": "assistant",
            "content": null,
            "tool_calls": [
                {"type": "function", "function": {"name": "list_dir", "arguments": {"path": "src"}}},
                {"id": "", "type": "function", "function": {"name": "glob"}}
            ]
        }}],
        "usage": {"prompt_tokens": 7, "completion_tokens": 3}
    });
    let c = parse_completion(&reply, 4).unwrap();
    assert_eq!(c.content, "");
    assert_eq!(c.tool_calls[0].id, "call_4_0");
    assert_eq!(c.tool_calls[0].arguments, json!({"path": "src"}));
    assert_eq!(c.tool_calls[1].id, "call_4_1");
    assert_eq!((c.tokens_in, c.tokens_out), (7, 3));
    assert_eq!(
        c.message,
        json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [
                {"id": "call_4_0", "type": "function",
                 "function": {"name": "list_dir", "arguments": "{\"path\":\"src\"}"}},
                {"id": "call_4_1", "type": "function",
                 "function": {"name": "glob", "arguments": "{}"}}
            ]
        })
    );
    assert!(parse_completion(&json!({"choices": []}), 1).is_err());
}

#[tokio::test]
async fn complete_posts_the_body_and_reads_the_reply() {
    let llm = Llm::start(vec![call("c1", "read_file", json!({"file_path": "a"}))]).await;
    let body = json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
    let c = client()
        .complete(
            &format!("{}/v1", llm.endpoint()),
            &body,
            1,
            &CancellationToken::new(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(c.tool_calls[0].name, "read_file");
    assert_eq!(c.tool_calls[0].arguments, json!("{\"file_path\":\"a\"}"));
    assert_eq!(llm.requests(), vec![body]);
}

#[tokio::test]
async fn server_errors_are_retried_then_reported() {
    let llm = Llm::start(vec![
        ResponseTemplate::new(503).set_body_string("loading"),
        text("ok"),
    ])
    .await;
    let c = client()
        .complete(&llm.endpoint(), &json!({}), 1, &CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(c.content, "ok");

    let llm = Llm::start(vec![
        ResponseTemplate::new(500).set_body_string("boom"),
        ResponseTemplate::new(500).set_body_string("boom"),
        ResponseTemplate::new(500).set_body_string("boom"),
    ])
    .await;
    let err = client()
        .complete(&llm.endpoint(), &json!({}), 1, &CancellationToken::new())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("500") && err.contains("boom") && err.contains("3 attempts"),
        "{err}"
    );
    assert_eq!(llm.requests().len(), 3);
}

#[tokio::test]
async fn client_errors_and_bad_bodies_fail_at_once() {
    let llm = Llm::start(vec![
        ResponseTemplate::new(404).set_body_json(json!({"error": "model 'x' not loaded"}))
    ])
    .await;
    let err = client()
        .complete(&llm.endpoint(), &json!({}), 1, &CancellationToken::new())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("404") && err.contains("not loaded"), "{err}");
    assert_eq!(llm.requests().len(), 1);

    let llm = Llm::start(vec![ResponseTemplate::new(200).set_body_string("<html>")]).await;
    let err = client()
        .complete(&llm.endpoint(), &json!({}), 1, &CancellationToken::new())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("invalid JSON"), "{err}");
}

#[tokio::test]
async fn unreachable_endpoint_is_reported_after_retries() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let err = client()
        .complete(
            &format!("http://127.0.0.1:{port}"),
            &json!({}),
            1,
            &CancellationToken::new(),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("unreachable") && err.contains("3 attempts"),
        "{err}"
    );
}

#[tokio::test]
async fn cancel_interrupts_a_slow_request() {
    let slow = completion(json!({"role": "assistant", "content": "late"}))
        .set_delay(Duration::from_secs(30));
    let llm = Llm::start(vec![slow]).await;
    let cancel = CancellationToken::new();
    let c = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        c.cancel();
    });
    let started = std::time::Instant::now();
    let out = client()
        .complete(&llm.endpoint(), &json!({}), 1, &cancel)
        .await
        .unwrap();
    assert!(out.is_none());
    assert!(started.elapsed() < Duration::from_secs(5));
}
