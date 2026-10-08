use serde_json::Value;

use crate::engine::EngineEvent;

const MAX_OUTPUT: usize = 64 * 1024;

/// Maps one line of `claude -p --output-format stream-json --verbose` output to events.
pub fn parse_line(line: &str) -> Vec<EngineEvent> {
    let line = line.trim();
    if line.is_empty() {
        return Vec::new();
    }
    let raw = || {
        vec![EngineEvent::Raw {
            line: line.to_string(),
        }]
    };
    let Ok(v) = serde_json::from_str::<Value>(line) else {
        return raw();
    };
    match v.get("type").and_then(Value::as_str) {
        Some("system") if v.get("subtype").and_then(Value::as_str) == Some("init") => {
            match v.get("session_id").and_then(Value::as_str) {
                Some(s) => vec![EngineEvent::SessionStarted {
                    session_id: s.to_string(),
                }],
                None => raw(),
            }
        }
        Some("assistant") => blocks(&v).iter().filter_map(assistant_block).collect(),
        Some("user") => blocks(&v).iter().filter_map(tool_result_block).collect(),
        Some("result") => result_events(&v),
        _ => raw(),
    }
}

fn blocks(v: &Value) -> Vec<Value> {
    v.pointer("/message/content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn assistant_block(b: &Value) -> Option<EngineEvent> {
    match b.get("type")?.as_str()? {
        "text" => Some(EngineEvent::AssistantText {
            text: b.get("text")?.as_str()?.to_string(),
        }),
        "tool_use" => Some(EngineEvent::ToolCall {
            id: str_field(b, "id"),
            tool: str_field(b, "name"),
            input: b.get("input").cloned().unwrap_or(Value::Null),
        }),
        _ => None,
    }
}

fn tool_result_block(b: &Value) -> Option<EngineEvent> {
    if b.get("type")?.as_str()? != "tool_result" {
        return None;
    }
    let output = match b.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    Some(EngineEvent::ToolResult {
        id: str_field(b, "tool_use_id"),
        output: truncate(output),
        is_error: b.get("is_error").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn truncate(mut s: String) -> String {
    if s.len() > MAX_OUTPUT {
        let mut cut = MAX_OUTPUT;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push_str("\n[truncated]");
    }
    s
}

fn result_events(v: &Value) -> Vec<EngineEvent> {
    let usage = v.get("usage");
    let n = |k: &str| {
        usage
            .and_then(|u| u.get(k))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let mut out = vec![EngineEvent::Usage {
        tokens_in: n("input_tokens")
            + n("cache_creation_input_tokens")
            + n("cache_read_input_tokens"),
        tokens_out: n("output_tokens"),
    }];
    let subtype = v
        .get("subtype")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let text = str_field(v, "result");
    let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
    if subtype == "success" && !is_error {
        out.push(EngineEvent::Finished { summary: text });
    } else if text.is_empty() {
        out.push(EngineEvent::Failed {
            error: subtype.to_string(),
        });
    } else {
        out.push(EngineEvent::Failed {
            error: format!("{subtype}: {text}"),
        });
    }
    out
}
