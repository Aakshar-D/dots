use dots_core::releases::{fetch_release, WITHDRAWN_MARKER};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TAG_PATH: &str = "/repos/Aakshar-D/dots/releases/tags/v0.3.0";

async fn serving(status: u16, body: serde_json::Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(TAG_PATH))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .expect(1)
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn a_release_reads_its_notes_and_is_not_withdrawn() {
    let server = serving(
        200,
        json!({
            "tag_name": "v0.3.0",
            "name": "Dots 0.3.0",
            "body": "• New",
            "html_url": "https://github.com/Aakshar-D/dots/releases/tag/v0.3.0",
            "prerelease": false
        }),
    )
    .await;
    let release = fetch_release(&server.uri(), "0.3.0")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(release.version, "0.3.0");
    assert_eq!(release.name, "Dots 0.3.0");
    assert_eq!(release.body, "• New");
    assert_eq!(
        release.url,
        "https://github.com/Aakshar-D/dots/releases/tag/v0.3.0"
    );
    assert!(!release.prerelease);
    assert!(!release.withdrawn);
}

#[tokio::test]
async fn a_prerelease_whose_body_starts_with_the_marker_is_withdrawn() {
    let server = serving(
        200,
        json!({
            "tag_name": "v0.3.0",
            "name": "Dots 0.3.0 (withdrawn)",
            "body": format!("{WITHDRAWN_MARKER}\n• New"),
            "html_url": "https://github.com/Aakshar-D/dots/releases/tag/v0.3.0",
            "prerelease": true
        }),
    )
    .await;
    let release = fetch_release(&server.uri(), "0.3.0")
        .await
        .unwrap()
        .unwrap();
    assert!(release.withdrawn);
    assert_eq!(release.body, "• New");
}

#[tokio::test]
async fn a_missing_release_is_none_and_a_server_error_is_an_error() {
    let missing = serving(404, json!({"message": "Not Found"})).await;
    assert!(fetch_release(&missing.uri(), "0.3.0")
        .await
        .unwrap()
        .is_none());
    let broken = serving(500, json!({"message": "oops"})).await;
    assert!(fetch_release(&broken.uri(), "0.3.0").await.is_err());
}
