//! A scripted OpenAI-compatible chat endpoint for local-engine tests: replies are served in
//! order, every request body is recorded, and an exhausted script answers 500.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

pub struct Llm {
    pub server: MockServer,
    requests: Arc<Mutex<Vec<Value>>>,
}

struct Script {
    replies: Mutex<VecDeque<ResponseTemplate>>,
    requests: Arc<Mutex<Vec<Value>>>,
}

impl Respond for Script {
    fn respond(&self, req: &Request) -> ResponseTemplate {
        let body = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
        self.requests.lock().unwrap().push(body);
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| ResponseTemplate::new(500).set_body_string("script exhausted"))
    }
}

impl Llm {
    pub async fn start(replies: Vec<ResponseTemplate>) -> Llm {
        let server = MockServer::start().await;
        let requests = Arc::new(Mutex::new(Vec::new()));
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(Script {
                replies: Mutex::new(replies.into()),
                requests: requests.clone(),
            })
            .mount(&server)
            .await;
        Llm { server, requests }
    }

    pub fn endpoint(&self) -> String {
        self.server.uri()
    }

    pub fn requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }

    /// Waits until at least `n` requests arrived.
    pub async fn wait_requests(&self, n: usize, secs: u64) -> Vec<Value> {
        let deadline = std::time::Instant::now() + Duration::from_secs(secs);
        loop {
            let reqs = self.requests();
            if reqs.len() >= n {
                return reqs;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "only {} of {n} requests arrived",
                reqs.len()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

/// A 200 chat completion carrying `message`, with 10 prompt and 5 completion tokens.
pub fn completion(message: Value) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "id": "chatcmpl-test",
        "object": "chat.completion",
        "choices": [{"index": 0, "message": message, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5}
    }))
}

pub fn text(content: &str) -> ResponseTemplate {
    completion(json!({"role": "assistant", "content": content}))
}

/// Tool calls `(id, name, arguments object)`, with arguments sent as a JSON string.
pub fn calls(list: &[(&str, &str, Value)]) -> ResponseTemplate {
    let calls: Vec<Value> = list
        .iter()
        .map(|(id, name, args)| {
            json!({
                "id": id,
                "type": "function",
                "function": {"name": name, "arguments": args.to_string()}
            })
        })
        .collect();
    completion(json!({"role": "assistant", "content": "", "tool_calls": calls}))
}

pub fn call(id: &str, name: &str, args: Value) -> ResponseTemplate {
    calls(&[(id, name, args)])
}

/// The messages array of a recorded request.
pub fn messages(req: &Value) -> Vec<Value> {
    req["messages"].as_array().cloned().unwrap_or_default()
}

/// The tool result message answering `id` in a recorded request.
pub fn tool_result(req: &Value, id: &str) -> String {
    messages(req)
        .iter()
        .find(|m| m["role"] == "tool" && m["tool_call_id"] == id)
        .unwrap_or_else(|| panic!("no tool result for {id} in {req:#}"))["content"]
        .as_str()
        .unwrap()
        .to_string()
}
