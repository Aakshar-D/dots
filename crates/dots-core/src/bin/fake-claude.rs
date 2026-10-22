//! Test stand-in for `claude -p --output-format stream-json`. Used only by dots-core tests.

use std::io::{Read, Write};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::process::{Child, Command};

/// Stays alive until `<dir>/stop` appears, `dir` is removed or 30 s pass, and holds a lock on
/// `<dir>/alive` all the while. The OS releases the lock when the process dies, so a test sees
/// whether it still runs without knowing its process id. `<dir>/ready` appears once the lock is
/// held.
fn linger(dir: &Path) {
    let alive = std::fs::File::create(dir.join("alive")).unwrap();
    alive.lock().unwrap();
    std::fs::write(dir.join("ready"), "").unwrap();
    let end = Instant::now() + Duration::from_secs(30);
    while Instant::now() < end && dir.exists() && !dir.join("stop").exists() {
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// This program again, in `mode` for `dir`, with no console window and no streams of its own.
/// It still inherits every handle of this process, as children on Windows do, so a lingering
/// process holds the output pipes of whoever started this one, like a real background process.
fn helper(mode: &str, dir: &str) -> Command {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args([mode, dir])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    dots_core::proc::hide_window(&mut cmd);
    cmd
}

/// Starts a lingering process and returns once it is ready. Stdin is read to its end first, so
/// whoever started this process can put it in a job before it has a child.
async fn start_lingerer(dir: &str) -> Child {
    std::io::stdin().read_to_end(&mut Vec::new()).ok();
    let child = helper("--linger", dir).spawn().unwrap();
    let ready = Path::new(dir).join("ready");
    let end = Instant::now() + Duration::from_secs(10);
    while !ready.exists() {
        assert!(Instant::now() < end, "the lingering process did not start");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    child
}

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
    if args.first().map(String::as_str) == Some("--version") {
        let version = std::env::var("FAKE_CLAUDE_VERSION").unwrap_or_else(|_| "2.1.295".into());
        println!("{version} (Claude Code)");
        return;
    }
    match (args.first().map(String::as_str), args.get(1)) {
        (Some("--linger"), Some(dir)) => return linger(Path::new(dir)),
        // Leaves a lingering child behind: a process whose parent has exited.
        (Some("--orphan"), Some(dir)) => {
            start_lingerer(dir).await;
            return;
        }
        // Stays for as long as its lingering child: a tree with every link alive.
        (Some("--tree"), Some(dir)) => {
            let _ = start_lingerer(dir).await.wait().await;
            return;
        }
        // Holds an orphan in a job and never lets go of it, as Dots does until it is killed.
        (Some("--hold-job"), Some(dir)) => {
            let mut child = helper("--orphan", dir)
                .stdin(Stdio::piped())
                .spawn()
                .unwrap();
            let _job = dots_core::proc::Job::assign(&child).unwrap();
            drop(child.stdin.take());
            let _ = child.wait().await;
            tokio::time::sleep(Duration::from_secs(30)).await;
            return;
        }
        _ => {}
    }
    let mut prompt = String::new();
    std::io::stdin().read_to_string(&mut prompt).ok();
    let mcp: Option<Value> = flag(&args, "--mcp-config")
        .map(|p| serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap());
    if let Ok(out) = std::env::var("FAKE_CLAUDE_ARGS_OUT") {
        // What the CLI would load: the config and the plugin folders are gone after the run.
        let plugin_dirs: Vec<Value> = args
            .windows(2)
            .filter(|w| w[0] == "--plugin-dir")
            .map(|w| json!({"path": w[1], "files": files_below(Path::new(&w[1]))}))
            .collect();
        let record = json!({
            "args": args,
            "pid": std::process::id(),
            "prompt": prompt,
            "cwd": std::env::current_dir().unwrap().to_string_lossy(),
            "mcp_tool_timeout": std::env::var("MCP_TOOL_TIMEOUT").ok(),
            // Every variable the CLI was started with, by name, and the values of those whose
            // name starts with `DOTS_TEST_` or `ANTHROPIC_DOTS_TEST_`.
            "env_names": std::env::vars().map(|(k, _)| k).collect::<Vec<_>>(),
            "env": std::env::vars()
                .filter(|(k, _)| {
                    k.starts_with("DOTS_TEST_") || k.starts_with("ANTHROPIC_DOTS_TEST_")
                })
                .collect::<std::collections::BTreeMap<_, _>>(),
            "mcp_config": mcp,
            "plugin_dirs": plugin_dirs,
        });
        std::fs::write(out, record.to_string()).unwrap();
    }
    let session = flag(&args, "--resume").unwrap_or_else(|| "fake-session".to_string());
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
        } else if let Some(dir) = step.get("orphan").and_then(Value::as_str) {
            // Done once the child has exited and has left its own child lingering in `dir`.
            assert!(helper("--orphan", dir).status().await.unwrap().success());
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
        } else if let Some(server) = step.get("mcp_list").and_then(Value::as_str) {
            let mcp = mcp.as_ref().expect("--mcp-config is required");
            let listed = gateway(mcp, server, "tools/list", json!({})).await;
            let names: Vec<Value> = listed["result"]["tools"]
                .as_array()
                .expect("a tool list")
                .iter()
                .map(|t| t["name"].clone())
                .collect();
            let line = json!({"type": "fake_mcp_tools", "server": server, "tools": names});
            writeln!(stdout, "{line}").unwrap();
            stdout.flush().unwrap();
        } else if let Some(call) = step.get("mcp_call") {
            let mcp = mcp.as_ref().expect("--mcp-config is required");
            let server = call["server"].as_str().expect("mcp_call.server");
            let tool = call["tool"].as_str().expect("mcp_call.tool");
            let id = format!("toolu_{i}");
            let use_line = json!({"type": "assistant", "message": {"content": [{
                "type": "tool_use", "id": id, "name": format!("mcp__{server}__{tool}"),
                "input": call["arguments"]}]}});
            writeln!(stdout, "{use_line}").unwrap();
            stdout.flush().unwrap();
            let params = json!({"name": tool, "arguments": call["arguments"]});
            let answer = gateway(mcp, server, "tools/call", params).await;
            let result = &answer["result"];
            let text: Vec<&str> = result["content"]
                .as_array()
                .map(|parts| parts.iter().filter_map(|p| p["text"].as_str()).collect())
                .unwrap_or_default();
            let line = json!({"type": "user", "message": {"role": "user", "content": [{
                "type": "tool_result", "tool_use_id": id, "content": text.join("\n"),
                "is_error": result["isError"].as_bool().unwrap_or(false)}]}});
            writeln!(stdout, "{line}").unwrap();
            stdout.flush().unwrap();
        }
    }
}

/// Every file below `dir`, as paths relative to it with `/`, sorted.
fn files_below(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&next) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let rel = path.strip_prefix(dir).unwrap();
                let parts: Vec<String> = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect();
                out.push(parts.join("/"));
            }
        }
    }
    out.sort();
    out
}

/// Sends `method` to the `--mcp-config` entry `server`, after `initialize`, as the CLI reaches
/// an MCP server over http. Returns the JSON-RPC answer.
async fn gateway(mcp: &Value, server: &str, method: &str, params: Value) -> Value {
    let entry = &mcp["mcpServers"][server];
    let url = entry["url"].as_str().expect("an http entry");
    let auth = entry["headers"]["Authorization"]
        .as_str()
        .expect("an auth header");
    let client = reqwest::Client::new();
    let post = |body: Value| {
        client
            .post(url)
            .header("Authorization", auth)
            .header("Accept", "application/json, text/event-stream")
            .json(&body)
            .send()
    };
    post(json!({"jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                   "clientInfo": {"name": "fake-claude", "version": "0"}}}))
    .await
    .expect("initialize");
    post(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .await
        .expect("initialized");
    post(json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
        .await
        .expect(method)
        .json()
        .await
        .expect("a JSON answer")
}
