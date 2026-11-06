//! The model catalog (model switcher spec section 2.1): the Claude list and a local endpoint's
//! `/v1/models`.

use dots_core::engine::local::client::models_url;
use dots_core::model::EngineKind;
use dots_core::models::{list_models, CLAUDE_MODELS, DEFAULT_CLAUDE_MODEL};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn claude_lists_auto_opus_sonnet_and_haiku_with_labels_and_hints() {
    let models = list_models(EngineKind::Claude, None).await.unwrap();
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["auto", "opus", "sonnet", "haiku"]);
    assert_eq!(models.len(), CLAUDE_MODELS.len() + 1);
    for m in &models {
        assert!(!m.label.is_empty() && !m.hint.is_empty(), "{m:?}");
    }
    assert_eq!(models[1].label, "Opus");
    assert!(ids.contains(&DEFAULT_CLAUDE_MODEL));
}

#[tokio::test]
async fn claude_ignores_an_endpoint() {
    let models = list_models(EngineKind::Claude, Some("not a url"))
        .await
        .unwrap();
    assert_eq!(models.len(), 4);
}

#[test]
fn models_url_accepts_v1_and_trailing_slashes() {
    for endpoint in [
        "http://127.0.0.1:1234",
        "http://127.0.0.1:1234/",
        "http://127.0.0.1:1234/v1",
        " http://127.0.0.1:1234/v1/ ",
    ] {
        assert_eq!(models_url(endpoint), "http://127.0.0.1:1234/v1/models");
    }
}

#[tokio::test]
async fn local_lists_the_ids_the_endpoint_lists() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "object": "list",
            "data": [
                {"id": "qwen2.5-coder-7b", "object": "model"},
                {"id": "llama-3.1-8b", "object": "model"},
                {"object": "model"}
            ]
        })))
        .mount(&server)
        .await;
    let models = list_models(EngineKind::Local, Some(&format!("{}/v1", server.uri())))
        .await
        .unwrap();
    let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["qwen2.5-coder-7b", "llama-3.1-8b"]);
    // A local model is shown by its id.
    assert_eq!(models[0].label, "qwen2.5-coder-7b");
}

#[tokio::test]
async fn local_needs_an_endpoint_that_passes_the_endpoint_check() {
    let none = list_models(EngineKind::Local, None).await.unwrap_err();
    assert!(none.to_string().contains("endpoint"), "{none}");
    let blank = list_models(EngineKind::Local, Some("  "))
        .await
        .unwrap_err();
    assert!(blank.to_string().contains("endpoint"), "{blank}");
    let https = list_models(EngineKind::Local, Some("https://example.com"))
        .await
        .unwrap_err();
    assert!(https.to_string().contains("http://"), "{https}");
}

#[tokio::test]
async fn local_says_what_went_wrong() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(404).set_body_string("no such route"))
        .mount(&server)
        .await;
    let e = list_models(EngineKind::Local, Some(&server.uri()))
        .await
        .unwrap_err()
        .to_string();
    // The status, not the reply's text.
    assert!(e.contains("404") && !e.contains("no such route"), "{e}");

    let garbled = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>hi</html>"))
        .mount(&garbled)
        .await;
    let e = list_models(EngineKind::Local, Some(&garbled.uri()))
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("model list") && !e.contains("<html>"), "{e}");

    // Nothing listens on port 1.
    let e = list_models(EngineKind::Local, Some("http://127.0.0.1:1"))
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("could not reach"), "{e}");
}

/// A server that counts the requests it gets, and one that redirects every request to it.
async fn redirect_pair(method_name: &str) -> (MockServer, MockServer) {
    let elsewhere = MockServer::start().await;
    Mock::given(method(method_name))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": "x"}]})))
        .mount(&elsewhere)
        .await;
    let bouncer = MockServer::start().await;
    Mock::given(method(method_name))
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("Location", format!("{}/v1/models", elsewhere.uri())),
        )
        .mount(&bouncer)
        .await;
    (bouncer, elsewhere)
}

#[tokio::test]
async fn local_does_not_follow_a_redirect() {
    let (bouncer, elsewhere) = redirect_pair("GET").await;
    let e = list_models(EngineKind::Local, Some(&bouncer.uri()))
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("307") && e.contains("redirect"), "{e}");
    assert!(elsewhere.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn the_endpoint_test_does_not_follow_a_redirect() {
    let (bouncer, elsewhere) = redirect_pair("POST").await;
    let e = dots_core::engine::local::probe(&bouncer.uri(), "m")
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("307") && e.contains("redirect"), "{e}");
    assert!(elsewhere.received_requests().await.unwrap().is_empty());
}
