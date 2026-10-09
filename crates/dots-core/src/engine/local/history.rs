//! Rebuilds a local-engine conversation from the `message` events of earlier runs.

use serde_json::{json, Value};

use crate::store::Store;
use crate::Result;

/// Tool result recorded for a tool call whose run stopped before it answered.
pub const INTERRUPTED: &str =
    "This tool call was interrupted when the run stopped; it may have partly run.";

/// The conversation of `run_id`'s ancestors (oldest first, following `parent_run_id`), built
/// from their `message` events and passed through `repair`.
pub async fn load(store: &Store, run_id: &str) -> Result<Vec<Value>> {
    let mut chain = Vec::new();
    let mut next = store.get_run(run_id).await?.parent_run_id;
    while let Some(id) = next {
        let run = store.get_run(&id).await?;
        next = run.parent_run_id.clone();
        chain.push(run.id);
    }
    chain.reverse();
    let mut messages = Vec::new();
    for id in chain {
        for ev in store.list_events(&id, 0).await? {
            if ev.kind == "message" {
                if let Some(m) = ev.data.get("message") {
                    messages.push(m.clone());
                }
            }
        }
    }
    Ok(repair(messages))
}

fn text_of(m: &Value) -> &str {
    m.get("content").and_then(Value::as_str).unwrap_or_default()
}

fn close_open(out: &mut Vec<Value>, open: &mut Vec<String>) {
    for id in open.drain(..) {
        out.push(json!({"role": "tool", "tool_call_id": id, "content": INTERRUPTED}));
    }
}

/// Makes a stored conversation valid to send again:
/// - every assistant tool call is followed by a tool result (a run that stopped mid-turn
///   leaves some unanswered; they get `INTERRUPTED`),
/// - tool results that answer no open call are dropped,
/// - consecutive user messages are merged, because some chat templates require roles to
///   alternate (a run that failed before the model answered leaves its prompt unanswered).
pub fn repair(messages: Vec<Value>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::with_capacity(messages.len());
    let mut open: Vec<String> = Vec::new();
    for m in messages {
        let role = m.get("role").and_then(Value::as_str).unwrap_or_default();
        if role == "tool" {
            let id = m
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if let Some(pos) = open.iter().position(|o| o == id) {
                open.remove(pos);
                out.push(m);
            }
            continue;
        }
        close_open(&mut out, &mut open);
        if role == "user" {
            if let Some(last) = out.last_mut().filter(|l| l["role"] == "user") {
                let merged = format!("{}\n\n{}", text_of(last), text_of(&m));
                last["content"] = Value::String(merged);
                continue;
            }
        }
        if role == "assistant" {
            open = m
                .get("tool_calls")
                .and_then(Value::as_array)
                .map(|calls| {
                    calls
                        .iter()
                        .filter_map(|c| c.get("id").and_then(Value::as_str).map(String::from))
                        .collect()
                })
                .unwrap_or_default();
        }
        out.push(m);
    }
    close_open(&mut out, &mut open);
    out
}
