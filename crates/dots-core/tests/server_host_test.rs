//! The Host check (accounts spec: API keys): the local server only answers requests addressed
//! to 127.0.0.1 or localhost on its own port, so a page on another name that was re-pointed at
//! 127.0.0.1 (DNS rebinding) cannot reach any route.

mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store};
use dots_core::approvals::ApprovalHub;
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::model::EngineKind;
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::server::{bind, host_allowed, serve, ServerState};
use dots_core::store::Store;
use reqwest::header::HOST;
use reqwest::StatusCode;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

const REFUSAL: &str = "This server only answers requests addressed to 127.0.0.1 or localhost.";

struct Srv {
    _dir: tempfile::TempDir,
    store: Store,
    port: u16,
    base: String,
    http: reqwest::Client,
}

async fn start() -> Srv {
    let (dir, store) = temp_store().await;
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let runner = Runner::new(
        store.clone(),
        bus,
        HashMap::<EngineKind, Arc<dyn Engine>>::new(),
        RunnerConfig {
            max_concurrent: 1,
            mcp_url: String::new(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(100),
            plugins_dir: dir.path().join("plugins"),
        },
    );
    let listener = bind(0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    serve(
        listener,
        ServerState {
            store: store.clone(),
            runner,
            hub,
        },
        CancellationToken::new(),
    );
    Srv {
        _dir: dir,
        store,
        port,
        base: format!("http://127.0.0.1:{port}"),
        http: reqwest::Client::new(),
    }
}

#[test]
fn host_allowed_table() {
    let p = 4321;
    assert!(host_allowed(Some("127.0.0.1:4321"), p));
    assert!(host_allowed(Some("localhost:4321"), p));
    assert!(host_allowed(Some("LOCALHOST:4321"), p));
    assert!(!host_allowed(Some("127.0.0.1"), p));
    assert!(!host_allowed(Some("127.0.0.1:4322"), p));
    assert!(!host_allowed(Some("evil.example:4321"), p));
    assert!(!host_allowed(Some("[::1]:4321"), p));
    assert!(!host_allowed(None, p));
    // The port must be the canonical digits, and the name exactly one of the two.
    assert!(!host_allowed(Some("127.0.0.1:04321"), p));
    assert!(!host_allowed(Some("127.0.0.1:+4321"), p));
    assert!(!host_allowed(Some("localhost.:4321"), p));
    assert!(!host_allowed(Some("a@127.0.0.1:4321"), p));
    assert!(!host_allowed(Some("127.0.0.1:"), p));
    assert!(!host_allowed(Some(""), p));
}

#[tokio::test]
async fn two_host_headers_are_refused() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let s = start().await;
    let send = |hosts: usize| {
        let port = s.port;
        async move {
            let mut c = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .unwrap();
            let mut req = String::from("GET /health HTTP/1.1\r\n");
            for _ in 0..hosts {
                req.push_str(&format!("Host: 127.0.0.1:{port}\r\n"));
            }
            req.push_str("Connection: close\r\n\r\n");
            c.write_all(req.as_bytes()).await.unwrap();
            let mut out = String::new();
            c.read_to_string(&mut out).await.unwrap();
            out
        }
    };
    assert!(send(1).await.starts_with("HTTP/1.1 200"));
    let two = send(2).await;
    assert!(two.starts_with("HTTP/1.1 403"), "{two}");
}

#[tokio::test]
async fn rebinding_host_is_refused_on_every_route() {
    let s = start().await;
    let evil = format!("evil.example:{}", s.port);
    let dot = s
        .store
        .create_dot(&spec("d"), None, "default")
        .await
        .unwrap();
    let trigger = format!("/dots/{}/trigger", dot.id);
    for (method, path) in [
        ("GET", "/health"),
        ("POST", trigger.as_str()),
        ("POST", "/mcp"),
        ("POST", "/mcp/s/x"),
        ("GET", "/cdp/nope"),
    ] {
        let r = s
            .http
            .request(method.parse().unwrap(), format!("{}{path}", s.base))
            .header(HOST, &evil)
            .body("{}")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{method} {path}");
        assert_eq!(r.text().await.unwrap(), "", "{path} has no body");
    }
    // /v1 gets the plain words as JSON, before any routing.
    for path in ["/v1/dots", "/v1/x", "/v1"] {
        let r = s
            .http
            .get(format!("{}{path}", s.base))
            .header(HOST, &evil)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{path}");
        let body: Value = r.json().await.unwrap();
        assert_eq!(body, json!({ "error": REFUSAL }), "{path}");
    }
    // A prefix that merely starts with /v1 is not /v1.
    let r = s
        .http
        .get(format!("{}/v1x", s.base))
        .header(HOST, &evil)
        .send()
        .await
        .unwrap();
    assert_eq!(r.text().await.unwrap(), "");
}

#[tokio::test]
async fn normal_callers_still_work() {
    let s = start().await;
    // reqwest's default Host is 127.0.0.1:<port>.
    let r = s
        .http
        .get(format!("{}/health", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    // localhost:<port> and a different case also pass.
    for host in [
        format!("localhost:{}", s.port),
        format!("LocalHost:{}", s.port),
    ] {
        let r = s
            .http
            .get(format!("{}/health", s.base))
            .header(HOST, host)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), StatusCode::OK);
    }
    // The webhook flow: an unknown token is the webhook's own answer, not the Host refusal.
    let dot = s
        .store
        .create_dot(&spec("w"), None, "default")
        .await
        .unwrap();
    let r = s
        .http
        .post(format!("{}/dots/{}/trigger", s.base, dot.id))
        .send()
        .await
        .unwrap();
    assert_ne!(r.status(), StatusCode::OK);
    // A websocket upgrade to the browser door with the right Host reaches the route (which
    // answers for its unknown secret itself).
    let r = s
        .http
        .get(format!("{}/cdp/bad-secret", s.base))
        .header(HOST, format!("127.0.0.1:{}", s.port))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", "dGhlIHNhbXBsZSBub25jZQ==")
        .send()
        .await
        .unwrap();
    let forged = s
        .http
        .get(format!("{}/cdp/bad-secret", s.base))
        .header(HOST, format!("evil.example:{}", s.port))
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .send()
        .await
        .unwrap();
    assert_eq!(forged.status(), StatusCode::FORBIDDEN);
    assert_ne!(
        r.status(),
        StatusCode::FORBIDDEN,
        "the Host check must not refuse the real caller"
    );
}
