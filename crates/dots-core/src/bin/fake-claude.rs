//! Test stand-in for `claude -p --output-format stream-json`. Used only by dots-core tests.

use std::io::{Read, Write};
use std::time::Duration;

use serde_json::{json, Value};

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

fn escape(s: &str) -> String {
    let quoted = serde_json::to_string(s).unwrap();
    quoted[1..quoted.len() - 1].to_string()
}

fn default_script() -> Vec<Value> {
    vec![
        json!({"emit": {"type": "system", "subtype": "init", "session_id": "$SESSION"}}),
        json!({"emit": {"type": "result", "subtype": "success", "is_error": false,
                        "result": "fake done", "usage": {"input_tokens": 1, "output_tokens": 1}}}),
    ]
}

async fn approve(mcp: &Value, req: &Value, step: usize) -> String {
    let dots = &mcp["mcpServers"]["dots"];
    let url = dots["url"].as_str().expect("dots url");
    let auth = dots["headers"]["Authorization"]
        .as_str()
        .expect("dots auth header");
    let client = reqwest::Client::new();
    client
        .post(url)
        .header("Authorization", auth)
        .json(&json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                       "clientInfo": {"name": "fake-claude", "version": "0"}}}))
        .send()
        .await
        .expect("initialize");
    let resp: Value = client
        .post(url)
        .header("Authorization", auth)
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "approve", "arguments": {
                "tool_name": req["tool_name"], "input": req["input"],
                "tool_use_id": format!("toolu_{step}")}}}))
        .send()
        .await
        .expect("tools/call")
        .json()
        .await
        .expect("json response");
    resp["result"]["content"][0]["text"]
        .as_str()
        .expect("decision text")
        .to_string()
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt).ok();
    if let Ok(out) = std::env::var("FAKE_CLAUDE_ARGS_OUT") {
        let record = json!({
            "args": args,
            "pid": std::process::id(),
            "prompt": prompt,
            "cwd": std::env::current_dir().unwrap().to_string_lossy(),
            "mcp_tool_timeout": std::env::var("MCP_TOOL_TIMEOUT").ok(),
        });
        std::fs::write(out, record.to_string()).unwrap();
    }
    let session = flag(&args, "--resume").unwrap_or_else(|| "fake-session".to_string());
    let mcp: Option<Value> = flag(&args, "--mcp-config")
        .map(|p| serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap());
    let script: Vec<Value> = match std::env::var("FAKE_CLAUDE_SCRIPT") {
        Ok(p) => serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap(),
        Err(_) => default_script(),
    };

    let mut stdout = std::io::stdout();
    for (i, step) in script.iter().enumerate() {
        if let Some(line) = step.get("emit") {
            let s = line
                .to_string()
                .replace("$SESSION", &session)
                .replace("$PROMPT", &escape(&prompt));
            writeln!(stdout, "{s}").unwrap();
            stdout.flush().unwrap();
        } else if let Some(ms) = step.get("sleep_ms").and_then(Value::as_u64) {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        } else if let Some(text) = step.get("stderr").and_then(Value::as_str) {
            eprintln!("{text}");
        } else if let Some(code) = step.get("exit").and_then(Value::as_i64) {
            std::process::exit(code as i32);
        } else if let Some(req) = step.get("approve") {
            let text = approve(mcp.as_ref().expect("--mcp-config is required"), req, i).await;
            let decision: Value = serde_json::from_str(&text).unwrap();
            let allowed = decision["behavior"] == "allow";
            let content = if allowed {
                "ok".to_string()
            } else {
                decision["message"].as_str().unwrap_or("denied").to_string()
            };
            let line = json!({"type": "user", "message": {"role": "user", "content": [{
                "type": "tool_result", "tool_use_id": format!("toolu_{i}"),
                "content": content, "is_error": !allowed}]}});
            writeln!(stdout, "{line}").unwrap();
            stdout.flush().unwrap();
        }
    }
}
