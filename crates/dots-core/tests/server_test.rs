mod common;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store};
use dots_core::approvals::{parked_message, ApprovalHub};
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::model::{Dot, EngineKind, NewRun, TriggerKind};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::server::{bind, serve, ServerState};
use dots_core::store::Store;
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

struct Srv {
    _dir: tempfile::TempDir,
    store: Store,
    runner: Arc<Runner>,
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
        },
    );
    let listener = bind(0).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    serve(
        listener,
        ServerState {
            store: store.clone(),
            runner: runner.clone(),
            hub,
        },
        CancellationToken::new(),
    );
    Srv {
        _dir: dir,
        store,
        runner,
        base: format!("http://127.0.0.1:{port}"),
        http: reqwest::Client::new(),
    }
}

async fn dot(srv: &Srv, name: &str, wait: u64) -> Dot {
    let mut s = spec(name);
    s.approval_wait_secs = wait;
    srv.store.create_dot(&s).await.unwrap()
}

#[tokio::test]
async fn health() {
    let srv = start().await;
    let r = srv
        .http
        .get(format!("{}/health", srv.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(r.text().await.unwrap(), "ok");
}

#[tokio::test]
async fn webhook_accepts_valid_requests() {
    let srv = start().await;
    let d = dot(&srv, "hook", 5).await;
    let url = format!("{}/dots/{}/trigger", srv.base, d.id);
    let r = srv
        .http
        .post(&url)
        .bearer_auth(&d.webhook_token)
        .json(&json!({"case": 7}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 202);
    let body: Value = r.json().await.unwrap();
    let run = srv
        .store
        .get_run(body["run_id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(run.trigger, TriggerKind::Webhook);
    assert_eq!(run.payload, Some(json!({"case": 7})));

    let r = srv
        .http
        .post(format!("{url}?token={}", d.webhook_token))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 202);
    let body: Value = r.json().await.unwrap();
    assert_eq!(
        srv.store
            .get_run(body["run_id"].as_str().unwrap())
            .await
            .unwrap()
            .payload,
        None
    );
}

#[tokio::test]
async fn webhook_rejects_bad_requests_without_creating_runs() {
    let srv = start().await;
    let d = dot(&srv, "hook", 5).await;
    let url = format!("{}/dots/{}/trigger", srv.base, d.id);
    let post =
        |b: reqwest::RequestBuilder| async move { b.send().await.unwrap().status().as_u16() };

    assert_eq!(post(srv.http.post(&url)).await, 401);
    assert_eq!(post(srv.http.post(&url).bearer_auth("wrong")).await, 401);
    assert_eq!(
        post(
            srv.http
                .post(format!("{}/dots/NOPE/trigger", srv.base))
                .bearer_auth("x")
        )
        .await,
        404
    );
    assert_eq!(
        post(
            srv.http
                .post(&url)
                .bearer_auth(&d.webhook_token)
                .body("not json")
        )
        .await,
        400
    );
    let big = "x".repeat(300 * 1024);
    assert_eq!(
        post(
            srv.http
                .post(&url)
                .bearer_auth(&d.webhook_token)
                .body(format!("\"{big}\""))
        )
        .await,
        413
    );
    srv.store.set_dot_enabled(&d.id, false).await.unwrap();
    assert_eq!(
        post(srv.http.post(&url).bearer_auth(&d.webhook_token)).await,
        409
    );
    assert!(srv
        .store
        .list_runs(Some(&d.id), 10)
        .await
        .unwrap()
        .is_empty());
}

async fn rpc(srv: &Srv, secret: Option<&str>, body: Value) -> reqwest::Response {
    let mut req = srv.http.post(format!("{}/mcp", srv.base)).json(&body);
    if let Some(s) = secret {
        req = req.bearer_auth(s);
    }
    req.send().await.unwrap()
}

fn decision(resp: &Value) -> Value {
    serde_json::from_str(resp["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[tokio::test]
async fn mcp_requires_a_run_secret() {
    let srv = start().await;
    let r = rpc(&srv, None, json!({"jsonrpc":"2.0","id":1,"method":"ping"})).await;
    assert_eq!(r.status(), 401);
    let r = rpc(
        &srv,
        Some("made-up"),
        json!({"jsonrpc":"2.0","id":1,"method":"ping"}),
    )
    .await;
    assert_eq!(r.status(), 401);
    let r = srv
        .http
        .get(format!("{}/mcp", srv.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 405);
}

#[tokio::test]
async fn mcp_handshake_and_approve_tool() {
    let srv = start().await;
    let d = dot(&srv, "gate", 0).await;
    let run = srv
        .store
        .create_run(&NewRun::new(&d.id, TriggerKind::Manual))
        .await
        .unwrap();
    let secret = srv.runner.register_secret(&run.id);
    let s = Some(secret.as_str());

    let init: Value = rpc(
        &srv,
        s,
        json!({"jsonrpc":"2.0","id":1,"method":"initialize",
        "params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"claude-code","version":"x"}}}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(init["result"]["serverInfo"]["name"], "dots");

    let r = rpc(
        &srv,
        s,
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    )
    .await;
    assert_eq!(r.status(), 202);

    let list: Value = rpc(
        &srv,
        s,
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(list["result"]["tools"][0]["name"], "approve");

    let allow: Value = rpc(
        &srv,
        s,
        json!({"jsonrpc":"2.0","id":3,"method":"tools/call",
        "params":{"name":"approve","arguments":{"tool_name":"Read","input":{"file_path":"a.txt"},"tool_use_id":"t1"}}}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(
        decision(&allow),
        json!({"behavior":"allow","updatedInput":{"file_path":"a.txt"}})
    );

    let deny: Value = rpc(
        &srv,
        s,
        json!({"jsonrpc":"2.0","id":4,"method":"tools/call",
        "params":{"name":"approve","arguments":{"tool_name":"Bash","input":{"command":"git push"}}}}),
    )
    .await
    .json()
    .await
    .unwrap();
    let approval = srv
        .store
        .approvals_for_run(&run.id)
        .await
        .unwrap()
        .remove(0);
    assert_eq!(
        decision(&deny),
        json!({"behavior":"deny","message": parked_message(&approval.id)})
    );

    let unknown: Value = rpc(
        &srv,
        s,
        json!({"jsonrpc":"2.0","id":5,"method":"resources/list"}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(unknown["error"]["code"], -32601);
}

#[tokio::test]
async fn mcp_accepts_large_tool_inputs() {
    let srv = start().await;
    let d = dot(&srv, "big", 0).await;
    let run = srv
        .store
        .create_run(&NewRun::new(&d.id, TriggerKind::Manual))
        .await
        .unwrap();
    let secret = srv.runner.register_secret(&run.id);
    let content = "x".repeat(300 * 1024);
    let r = rpc(
        &srv,
        Some(&secret),
        json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"approve","arguments":{"tool_name":"Write",
            "input":{"file_path":"a.txt","content": content}}}}),
    )
    .await;
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.unwrap();
    assert_eq!(decision(&body)["behavior"], "allow");
}
