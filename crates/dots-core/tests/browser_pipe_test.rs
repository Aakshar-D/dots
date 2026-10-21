//! A dot's Edge over its debugging pipe (browser spec section 14), with the test playing Edge:
//! messages put together from the pieces a pipe delivers; Dots' own requests and Playwright's
//! under ids that never collide; events to Playwright; what Edge attached for a Playwright
//! undone when it goes; garbage, huge messages, and Edge closing mid-relay.

mod common;

use std::collections::HashSet;
use std::time::Duration;

use common::edge_pipe::TestEdge;
use dots_core::browser::host::HostError;
use dots_core::browser::pipe::{Frames, MAX_MESSAGE};
use serde_json::{json, Value};

fn text(frames: Vec<Vec<u8>>) -> Vec<String> {
    frames
        .into_iter()
        .map(|f| String::from_utf8(f).unwrap())
        .collect()
}

#[test]
fn messages_are_put_together_from_any_pieces() {
    assert_eq!(MAX_MESSAGE, 256 * 1024 * 1024);
    let mut frames = Frames::default();
    // Cut anywhere, several at once, and empty ones between.
    assert!(frames.push(br#"{"id":1,"res"#).is_empty());
    assert_eq!(
        text(frames.push(b"ult\":{}}\0\0{\"id\":2}\0{\"id\"")),
        [r#"{"id":1,"result":{}}"#, r#"{"id":2}"#]
    );
    assert_eq!(text(frames.push(b":3}\0")), [r#"{"id":3}"#]);
    assert!(frames.push(b"\0\0").is_empty());
    // One longer than the most allowed is dropped up to its NUL, in whatever pieces; the next
    // one is read.
    let mut small = Frames::with_max(8);
    assert!(small.push(b"12345").is_empty());
    assert!(small.push(b"6789").is_empty());
    assert!(small.push(b"0123").is_empty());
    assert_eq!(text(small.push(b"45\0short\0")), ["short"]);
    assert_eq!(text(small.push(b"12345678\0")), ["12345678"]);
    assert_eq!(text(small.push(b"123456789\0ok\0")), ["ok"]);
}

#[tokio::test]
async fn dots_and_playwright_get_their_own_answers_whatever_the_ids() {
    let mut edge = TestEdge::start();
    let (serial, mut playwright) = edge.pipe.connect().unwrap();
    // Playwright numbers its requests from 1, as Dots' pipe does.
    assert!(edge.pipe.relay(
        serial,
        r#"{"id":1,"method":"Page.navigate","sessionId":"S1","params":{"url":"https://shop.com/"}}"#
    ));
    let pipe = edge.pipe.clone();
    let pages = tokio::spawn(async move { pipe.pages().await });
    assert!(edge
        .pipe
        .relay(serial, r#"{"id":2,"method":"Browser.getVersion"}"#));
    let mut sent = vec![edge.next().await, edge.next().await, edge.next().await];
    let ids: HashSet<u64> = sent.iter().map(|m| m["id"].as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 3, "{sent:?}");
    sent.sort_by_key(|m| m["method"].as_str().unwrap().to_string());
    let [version, navigate, targets] = [sent[0].clone(), sent[1].clone(), sent[2].clone()];
    assert_eq!(version["method"], "Browser.getVersion");
    assert_eq!(navigate["sessionId"], "S1");
    assert_eq!(navigate["params"]["url"], "https://shop.com/");
    assert_eq!(targets["method"], "Target.getTargets");
    // Answered out of order; an answer under an id nobody asked with goes nowhere.
    edge.send(&json!({"id": 999_999, "result": {}})).await;
    edge.send(&json!({"id": version["id"], "result": {"product": "Edg/1"}}))
        .await;
    edge.send(&json!({"id": targets["id"], "result": {"targetInfos": [
        {"targetId": "P1", "type": "page", "url": "https://shop.com/cart"},
        {"targetId": "SW", "type": "service_worker", "url": "chrome-extension://x/bg.js"}
    ]}}))
    .await;
    edge.send(&json!({"id": navigate["id"], "sessionId": "S1", "result": {"frameId": "F1"}}))
        .await;
    assert_eq!(
        pages.await.unwrap().unwrap(),
        ["https://shop.com/cart".to_string()]
    );
    let first: Value = serde_json::from_str(&playwright.recv().await.unwrap()).unwrap();
    let second: Value = serde_json::from_str(&playwright.recv().await.unwrap()).unwrap();
    assert_eq!(first, json!({"id": 2, "result": {"product": "Edg/1"}}));
    assert_eq!(
        second,
        json!({"id": 1, "sessionId": "S1", "result": {"frameId": "F1"}})
    );
    // Events go to Playwright as they came; an error answer to Dots is an error.
    edge.send(&json!({"method": "Page.loadEventFired", "sessionId": "S1", "params": {}}))
        .await;
    assert_eq!(
        serde_json::from_str::<Value>(&playwright.recv().await.unwrap()).unwrap()["method"],
        "Page.loadEventFired"
    );
    let pipe = edge.pipe.clone();
    let failing = tokio::spawn(async move { pipe.ask("Browser.close", json!({})).await });
    let close = edge.next().await;
    edge.send(&json!({"id": close["id"], "error": {"code": -32000, "message": "no"}}))
        .await;
    assert!(matches!(
        failing.await.unwrap(),
        Err(HostError::Unreachable(_))
    ));
}

#[tokio::test]
async fn what_edge_attached_for_a_playwright_is_undone_when_it_goes() {
    let mut edge = TestEdge::start();
    let (first, mut old) = edge.pipe.connect().unwrap();
    for (session, method) in [
        ("S1", "Target.attachedToTarget"),
        ("S2", "Target.attachedToTarget"),
        ("S3", "Target.attachedToTarget"),
        ("S2", "Target.detachedFromTarget"),
    ] {
        edge.send(&json!({"method": method, "params": {"sessionId": session}}))
            .await;
    }
    // A worker attached inside a page's session is that session's, not a top-level one.
    edge.send(
        &json!({"method": "Target.attachedToTarget", "sessionId": "S1",
                      "params": {"sessionId": "W1"}}),
    )
    .await;
    for _ in 0..5 {
        old.recv().await.unwrap();
    }
    // A new Playwright comes before the old one's end was seen: the old one is let go, and
    // its sessions are detached before the new one's first request.
    let (second, mut new) = edge.pipe.connect().unwrap();
    assert!(old.recv().await.is_none(), "the old connection ends");
    assert!(!edge
        .pipe
        .relay(first, r#"{"id":5,"method":"Browser.getVersion"}"#));
    assert!(edge
        .pipe
        .relay(second, r#"{"id":1,"method":"Target.setAutoAttach"}"#));
    let undone: Vec<Value> = vec![edge.next().await, edge.next().await, edge.next().await];
    assert_eq!(undone[0]["method"], "Target.detachFromTarget");
    assert_eq!(undone[0]["params"]["sessionId"], "S1");
    assert_eq!(undone[1]["params"]["sessionId"], "S3");
    assert_eq!(undone[2]["method"], "Target.setAutoAttach");
    assert_eq!(
        undone[2]["params"],
        json!({"autoAttach": false, "waitForDebuggerOnStart": false})
    );
    // Their answers go nowhere.
    for message in &undone {
        edge.send(&json!({"id": message["id"], "result": {}})).await;
    }
    let request = edge.next().await;
    assert_eq!(request["method"], "Target.setAutoAttach");
    edge.send(&json!({"id": request["id"], "result": {}})).await;
    assert_eq!(new.recv().await.unwrap(), r#"{"id":1,"result":{}}"#);
    // The one that went is undone once; a stale serial changes nothing.
    edge.pipe.disconnect(first);
    edge.pipe.disconnect(second);
    let last = edge.next().await;
    assert_eq!(last["params"]["autoAttach"], false);
    assert!(edge.next_within(Duration::from_millis(200)).await.is_none());
}

#[tokio::test]
async fn garbage_is_dropped_and_huge_messages_arrive_whole() {
    let mut edge = TestEdge::start();
    let (serial, mut playwright) = edge.pipe.connect().unwrap();
    // From Playwright: not JSON (a raw NUL in it), no id, not an object.
    assert!(!edge
        .pipe
        .relay(serial, "{\"id\":7,\"method\":\"Fake\0.x\"}"));
    assert!(!edge
        .pipe
        .relay(serial, r#"{"method":"Browser.getVersion"}"#));
    assert!(!edge.pipe.relay(serial, "[1,2]"));
    assert!(!edge.pipe.relay(serial, r#"{"id":"x","method":"A.b"}"#));
    assert!(edge.next_within(Duration::from_millis(200)).await.is_none());
    // A NUL inside a JSON string is escaped on the way: the pipe's framing holds.
    assert!(edge.pipe.relay(
        serial,
        r#"{"id":8,"method":"Runtime.evaluate","params":{"expression":"'\u0000'"}}"#
    ));
    let evaluate = edge.next().await;
    assert_eq!(evaluate["params"]["expression"], "'\u{0}'");
    // From Edge: not JSON, not an object; then a 20 MB answer in three pieces.
    edge.send_raw(b"not json\0[1]\0").await;
    let big = format!(
        "{{\"id\":{},\"result\":{{\"data\":\"{}\"}}}}\0",
        evaluate["id"],
        "x".repeat(20_000_000)
    );
    let (a, rest) = big.as_bytes().split_at(7);
    let (b, c) = rest.split_at(10_000_000);
    edge.send_raw(a).await;
    edge.send_raw(b).await;
    edge.send_raw(c).await;
    let answer: Value = serde_json::from_str(&playwright.recv().await.unwrap()).unwrap();
    assert_eq!(answer["id"], 8);
    assert_eq!(answer["result"]["data"].as_str().unwrap().len(), 20_000_000);
}

#[tokio::test]
async fn edge_closing_mid_relay_ends_every_wait() {
    let mut edge = TestEdge::start();
    let (serial, mut playwright) = edge.pipe.connect().unwrap();
    assert!(edge
        .pipe
        .relay(serial, r#"{"id":1,"method":"Page.navigate"}"#));
    let pipe = edge.pipe.clone();
    let pages = tokio::spawn(async move { pipe.pages().await });
    edge.next().await;
    edge.next().await;
    // Half a message, then Edge is gone.
    edge.send_raw(br#"{"id":1,"res"#).await;
    edge.close().await;
    assert!(matches!(
        pages.await.unwrap(),
        Err(HostError::Unreachable(_))
    ));
    assert!(playwright.recv().await.is_none());
    edge.pipe.closed().await;
    assert!(edge.pipe.is_closed());
    assert!(edge.pipe.connect().is_none());
    assert!(!edge
        .pipe
        .relay(serial, r#"{"id":2,"method":"Browser.getVersion"}"#));
    assert!(matches!(
        edge.pipe.ask("Browser.getVersion", json!({})).await,
        Err(HostError::Unreachable(_))
    ));
}
