//! The door of a run's Playwright to its dot's Edge (browser spec section 14), on a server of
//! the test's own with the door's route, and the test playing Edge: one connection per secret
//! (two clients racing, a reconnection, a wrong secret, a request that is not a WebSocket), the
//! relay both ways with large messages, what Playwright set up undone when it goes, and Edge
//! closing mid-relay.

mod common;

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::WebSocketUpgrade;
use axum::extract::Path;
use axum::routing::get;
use axum::Router;
use common::edge_pipe::TestEdge;
use common::fake_browser::{cart_script, exe, host, playwright_command, FakeEdges, CART};
use common::fake_mcp::FakeMcp;
use common::{spec, temp_store};
use dots_core::approvals::ApprovalHub;
use dots_core::browser::door::{self, new_secret, Doors};
use dots_core::browser::pipe::MAX_MESSAGE;
use dots_core::browser::{Browsers, CallLimit, CDP_ENDPOINT_VAR};
use dots_core::events::new_bus;
use dots_core::model::{NewRun, TriggerKind, WorkspaceMode};
use dots_core::settings::Settings;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};
use tokio_util::sync::CancellationToken;

/// A server with the door's route on `listener`, as Dots' own server has it.
fn serve_on(listener: tokio::net::TcpListener, doors: Arc<Doors>) {
    let app = Router::new().route(
        "/cdp/{secret}",
        get(
            move |Path(secret): Path<String>,
                  upgrade: Result<WebSocketUpgrade, WebSocketUpgradeRejection>| {
                door::connect(doors.clone(), secret, upgrade)
            },
        ),
    );
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
}

/// `serve_on` a port of its own; its port.
async fn serve(doors: Arc<Doors>) -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    serve_on(listener, doors);
    port
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Connects as Playwright does, taking messages of up to 256 MB.
async fn enter(port: u16, secret: &str) -> Result<Socket, u16> {
    let url = format!("ws://127.0.0.1:{port}/cdp/{secret}");
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    match tokio_tungstenite::connect_async_with_config(url, Some(config), true).await {
        Ok((socket, _)) => Ok(socket),
        Err(WsError::Http(answer)) => Err(answer.status().as_u16()),
        Err(e) => panic!("{e}"),
    }
}

/// The next message from Edge as Playwright reads it.
async fn next(socket: &mut Socket) -> Value {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .expect("a message within 10 s")
            .expect("the socket is open")
            .unwrap();
        if let Message::Text(text) = message {
            return serde_json::from_str(text.as_str()).unwrap();
        }
    }
}

async fn send(socket: &mut Socket, message: Value) {
    socket
        .send(Message::Text(message.to_string().into()))
        .await
        .unwrap();
}

/// Whether the door of `secret` is spent within 5 s.
async fn spent_within(doors: &Doors, secret: &str) -> bool {
    for _ in 0..250 {
        if doors.spent(secret) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[test]
fn a_secret_is_128_random_bits() {
    let a = new_secret();
    assert_eq!(a.len(), 22);
    assert!(a
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    assert_ne!(a, new_secret());
}

#[tokio::test]
async fn a_secret_lets_one_connection_in_once() {
    let edge = TestEdge::start();
    let doors = Arc::new(Doors::default());
    let port = serve(doors.clone()).await;
    let secret = doors.open(edge.pipe.clone());
    // Two clients race for one secret: exactly one comes in.
    let (a, b) = tokio::join!(enter(port, &secret), enter(port, &secret));
    let (mut inside, refused) = match (a, b) {
        (Ok(socket), Err(status)) | (Err(status), Ok(socket)) => (socket, status),
        (a, b) => panic!("one of two, not {:?} and {:?}", a.is_ok(), b.is_ok()),
    };
    assert_eq!(refused, 404);
    assert!(!doors.spent(&secret), "in use, not spent");
    assert_eq!(enter(port, "not-the-secret").await.err(), Some(404));
    // Playwright reconnects after its connection dropped: the secret is spent.
    inside.close(None).await.unwrap();
    assert!(spent_within(&doors, &secret).await);
    assert_eq!(enter(port, &secret).await.err(), Some(404));
    doors.shut(&secret);
    assert!(!doors.spent(&secret));
    assert_eq!(enter(port, &secret).await.err(), Some(404));
    // A request that is not a WebSocket spends the door too.
    let plain = doors.open(edge.pipe.clone());
    let answer = reqwest::get(format!("http://127.0.0.1:{port}/cdp/{plain}"))
        .await
        .unwrap();
    assert!(answer.status().is_client_error());
    assert!(doors.spent(&plain));
    assert_eq!(enter(port, &plain).await.err(), Some(404));
}

#[tokio::test]
async fn playwright_and_edge_talk_through_the_door() {
    let mut edge = TestEdge::start();
    let doors = Arc::new(Doors::default());
    let port = serve(doors.clone()).await;
    let secret = doors.open(edge.pipe.clone());
    let mut playwright = enter(port, &secret).await.unwrap();
    send(
        &mut playwright,
        json!({"id": 1, "method": "Target.setAutoAttach",
               "params": {"autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true}}),
    )
    .await;
    let request = edge.next().await;
    assert_eq!(request["method"], "Target.setAutoAttach");
    edge.send(&json!({"method": "Target.attachedToTarget",
                      "params": {"sessionId": "S1", "targetInfo": {"type": "page"}}}))
        .await;
    edge.send(&json!({"id": request["id"], "result": {}})).await;
    assert_eq!(next(&mut playwright).await["params"]["sessionId"], "S1");
    assert_eq!(next(&mut playwright).await, json!({"id": 1, "result": {}}));
    // Large messages pass both ways: 20 MB from Playwright, 30 MB from Edge.
    let text = "y".repeat(20_000_000);
    send(
        &mut playwright,
        json!({"id": 2, "sessionId": "S1", "method": "Runtime.evaluate",
               "params": {"expression": text}}),
    )
    .await;
    let evaluate = edge.next().await;
    assert_eq!(
        evaluate["params"]["expression"].as_str().unwrap().len(),
        20_000_000
    );
    edge.send(&json!({"id": evaluate["id"], "sessionId": "S1",
                      "result": {"data": "z".repeat(30_000_000)}}))
        .await;
    let answer = next(&mut playwright).await;
    assert_eq!(answer["id"], 2);
    assert_eq!(answer["result"]["data"].as_str().unwrap().len(), 30_000_000);
    // Playwright goes: the session Edge attached for it is detached.
    playwright.close(None).await.unwrap();
    let detach = edge.next().await;
    assert_eq!(detach["method"], "Target.detachFromTarget");
    assert_eq!(detach["params"]["sessionId"], "S1");
    assert_eq!(edge.next().await["params"]["autoAttach"], false);
    assert!(spent_within(&doors, &secret).await);
}

#[tokio::test]
async fn edge_closing_mid_relay_ends_the_connection() {
    let mut edge = TestEdge::start();
    let doors = Arc::new(Doors::default());
    let port = serve(doors.clone()).await;
    let secret = doors.open(edge.pipe.clone());
    let mut playwright = enter(port, &secret).await.unwrap();
    send(&mut playwright, json!({"id": 1, "method": "Page.navigate"})).await;
    edge.next().await;
    edge.close().await;
    let end = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match playwright.next().await {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(end.is_ok(), "the socket closes with Edge");
    assert!(spent_within(&doors, &secret).await);
    // A door to an Edge that has gone lets nobody in for long.
    let late = doors.open(edge.pipe.clone());
    if let Ok(mut socket) = enter(port, &late).await {
        let closed = tokio::time::timeout(Duration::from_secs(10), socket.next()).await;
        assert!(matches!(
            closed,
            Ok(Some(Ok(Message::Close(_))) | None | Some(Err(_)))
        ));
    }
}

/// Playwright's connection dropped and it would come back with the same secret: the door is
/// spent, so the run's next browser call starts a new Playwright server with a new door.
#[tokio::test]
async fn a_run_whose_playwright_lost_its_door_gets_a_new_one() {
    let (dir, store) = temp_store().await;
    store
        .put_settings(&Settings {
            edge_path: Some(exe()),
            ..Settings::default()
        })
        .await
        .unwrap();
    let root = dir.path().join("browser");
    let edges = FakeEdges::new(&root);
    let playwright = FakeMcp::new(cart_script());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let browsers = Browsers::new(
        store.clone(),
        Arc::new(host(&root)),
        playwright_command(&playwright),
        port,
    );
    serve_on(listener, browsers.doors().clone());
    let mut s = spec("shopper");
    s.workdir = dir.path().to_string_lossy().into_owned();
    s.workspace_mode = WorkspaceMode::Folder;
    s.browser = true;
    let dot = store.create_dot(&s, None, "default").await.unwrap();
    edges.first_pages(&dot.id, &[CART]);
    let run = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let browser = browsers.for_run(&dot, &run.id, dir.path());
    let hub = ApprovalHub::new(store.clone(), new_bus());
    let snapshot = || async {
        browser
            .call(
                &hub,
                "browser_snapshot",
                json!({}),
                CallLimit::PerformWithin(Duration::from_secs(30)),
                &CancellationToken::new(),
            )
            .await
    };
    let endpoints = || -> Vec<String> {
        playwright
            .log()
            .iter()
            .filter_map(|r| r.get("start"))
            .map(|start| start["env"][CDP_ENDPOINT_VAR].as_str().unwrap().to_string())
            .collect()
    };
    assert_ne!(snapshot().await["isError"], true);
    let first = endpoints().pop().unwrap();
    let secret = first.rsplit('/').next().unwrap().to_string();
    // Playwright comes in through its door, and its connection drops.
    let (mut socket, _) = tokio_tungstenite::connect_async(&first).await.unwrap();
    socket.close(None).await.unwrap();
    assert!(spent_within(browsers.doors(), &secret).await);
    assert_eq!(enter(port, &secret).await.err(), Some(404));
    // The next call starts a new server, with a door of its own; the old secret stays out.
    assert_ne!(snapshot().await["isError"], true);
    let now = endpoints();
    assert_eq!(now.len(), 2);
    assert_ne!(now[1], first);
    assert_eq!(enter(port, &secret).await.err(), Some(404));
    let mut again = enter(port, now[1].rsplit('/').next().unwrap())
        .await
        .unwrap();
    assert_eq!(edges.starts(&dot.id).len(), 1, "the same Edge");
    // The run ends: its Edge closes, and with it the connection.
    browser.close().await;
    assert!(edges.gone_within(&dot.id, Duration::from_secs(10)).await);
    let closed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match again.next().await {
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                Some(Ok(_)) => {}
            }
        }
    })
    .await;
    assert!(closed.is_ok());
}
