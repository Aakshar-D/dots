# Dots Local Engine (Plan 2 of 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the local engine to `dots-core`: a tool loop against an OpenAI-compatible chat endpoint (LM Studio, Ollama) with seven built-in, workspace-confined tools, sharing the Claude engine's policy, approval, park and resume path, plus the "Test endpoint" probe the Plan 3 UI will call.

**Architecture:** `LocalEngine` implements the existing `Engine` trait. Per run it sends the conversation plus tool schemas to `POST {endpoint}/v1/chat/completions` (non-streaming), and for each tool call: `tools::prepare` (known tool, JSON-object arguments, required arguments, paths inside the workspace) → `PermissionGate::check` (the same `ApprovalHub` the Claude MCP endpoint uses) → `tools::run`. Every chat message is recorded as a `message` run event; a resume or chat follow-up rebuilds the conversation from the `message` events of its ancestor runs (`history::load`), so the runner's engine-agnostic park/resume flow works unchanged. `Runtime` registers the engine and exposes `test_endpoint`.

**Tech Stack:** Rust 1.95 (edition 2021), tokio, reqwest 0.12 (no TLS), serde_json, `ignore` 0.4 (gitignore-aware walking), `globset` 0.4, `regex` 1; tests add `wiremock` 0.6.

**Spec:** `docs/superpowers/specs/2026-10-08-dots-design.md` — §5 "Local engine" (pinned on 2026-10-09 in commit `b34d1f8`), §8 rows "Local endpoint unreachable / 5xx" and "Model returns malformed tool call", §9 "Local engine".

**Previous plan:** `docs/superpowers/plans/2026-10-08-dots-core.md` (Plan 1, done). **Next:** Plan 3 — Tauri app + React UI (spec §7, phases 5–7).

**How this plan's code was checked:** the code blocks below were generated from a throwaway worktree of commit `841b6ae` where they compiled and passed on Windows 11: after all tasks, `cargo test -p dots-core` passed 179 tests with 0 failures, `cargo clippy --all-targets` reported no warnings and `cargo fmt --check` was clean. The intermediate states after Task 2 (7 + 9 tests pass, one expected warning) and after Task 7 (all suites pass, no clippy warnings) were checked the same way.

## Global Constraints

- Toolchain: Rust 1.95 stable, edition 2021. Primary OS Windows 11; code must also compile on Linux/macOS (`#[cfg(windows)]` for Windows-only bits). `git` must be on PATH.
- Repo root: `C:\Users\aksha\dots`. Cargo workspace with one member: `crates/dots-core`. All commands below run from the repo root.
- `sqlx` runtime queries only (`sqlx::query(...).bind(...)`); this plan adds no SQL and no migration.
- All timestamps: RFC 3339 UTC with milliseconds, produced only by `util::now()`.
- Subscription rule: dots only ever spawn the official `claude` CLI and never read, copy or send CLI credentials. The local engine talks only to the user-configured `endpoint_url`.
- Local endpoints are plain `http://` URLs (no TLS support compiled in). `{endpoint}`, `{endpoint}/` and `{endpoint}/v1` all mean `{endpoint}/v1/chat/completions`.
- Built-in tools and their policy aliases: `read_file`→`Read`, `write_file`→`Write`, `edit_file`→`Edit`, `list_dir`→`LS`, `glob`→`Glob`, `grep`→`Grep`, `shell`→`Bash`. Argument names: `file_path`, `content`, `old_string`, `new_string`, `path`, `pattern`, `glob`, `ignore_case`, `offset`, `limit`, `command`, `timeout`.
- Limits: tool output to the model ≤ 30 KB (30720 bytes); `list_dir`/`glob` ≤ 500 entries; `grep` ≤ 200 matches, lines cut at 300 chars, files > 2 MB skipped; `shell` timeout default 120 s, max 600 s; endpoint retries: 2, after 1 s and 2 s; `Runtime::test_endpoint` waits ≤ 120 s.
- Dependencies are added once in Task 1; no later task edits `Cargo.toml`.
- Commit messages: Conventional Commits; no AI co-author / attribution trailers.

### Deliberate deviations from the spec (agreed simplifications)

| Spec | Plan 2 | Why |
|---|---|---|
| §5: output cap 30 KB on `shell` | Cap applies to every tool's output | Protects small local context windows; same limit everywhere |
| §5: "Test endpoint" lives in Settings | Core function `Runtime::test_endpoint` now; the button is Plan 3 | Plan 3's UI only calls it |
| §9 manual smoke: scheduled + webhook + approval run for a local dot | Plan 2 smoke: manual run, live approval, park + resume against LM Studio | Schedule/webhook triggers are engine-agnostic and already tested; the full trigger smoke is Plan 3 phase 7 |
| — | `read_tail` moves from `engine/claude/mod.rs` to `proc.rs` | The `shell` tool needs the same bounded pipe reader |

## Review Focus

1. **Paths that escape the workspace** — `..`, absolute paths elsewhere, a junction/symlink inside the workspace pointing out, NTFS stream names (`README.md:hidden`) — must be refused before the policy sees them and never touched. → Task 2 `confine_rejects_escapes`; Task 7 `denied_and_malformed_calls_are_reported_to_the_model` (`../secret`).
2. **Malformed model output** — arguments as a JSON string, as an object, empty, invalid JSON, a non-object, an unknown tool, a missing or empty tool-call id — must become an error result for the model (or a synthesized `call_<turn>_<i>` id) and the run must continue. → Task 2 `prepare_validates_calls_before_the_policy`; Task 5 `parse_completion_normalizes_tool_calls`; Task 7 `denied_and_malformed_calls_are_reported_to_the_model`.
3. **A run that stops mid-turn** (cancel during a tool or a pending approval, endpoint failure before the model answered) must not poison the next resume or chat follow-up: unanswered tool calls get a synthetic result, orphan results are dropped, consecutive user messages are merged. → Task 6 `repair_*`, `load_follows_the_parent_chain_in_order`.
4. **Windows realities** — CRLF files edited with LF `old_string`, PowerShell exit codes / quoting / UTF-8 output, a command that never ends, huge output — must behave like a user expects (edit succeeds, exit code reported, killed at timeout or cancel, output capped). → Task 2 `edit_file_matches_lf_text_in_crlf_files`; Task 4 `shell_*`.
5. **Endpoint misconfiguration** — `/v1` suffix or trailing slash, `https://`, server down, model not loaded (4xx with a message), HTML instead of JSON — must give a clear error naming the cause. → Task 1 `invalid_specs_are_rejected`; Task 5 client tests; Task 8 `test_endpoint_checks_for_a_well_formed_tool_call`.

## File Map

```
crates/dots-core/
  Cargo.toml                          + globset, ignore, regex; dev: wiremock          (Task 1)
  src/model.rs                        check_endpoint_url + DotSpec validation           (Task 1)
  src/proc.rs                         read_tail moved here                              (Task 4)
  src/engine/mod.rs                   pub mod local; EngineEvent::Message               (Tasks 2, 7)
  src/engine/claude/mod.rs            uses proc::read_tail                              (Task 4)
  src/engine/local/mod.rs             LocalEngine, Session loop, probe                  (Tasks 2, 5-8)
  src/engine/local/tools/mod.rs       TOOLS, prepare, run, confine, caps, schemas       (Tasks 2-4)
  src/engine/local/tools/fs.rs        read_file, write_file, edit_file, list_dir        (Task 2)
  src/engine/local/tools/search.rs    glob, grep                                        (Task 3)
  src/engine/local/tools/shell.rs     shell                                             (Task 4)
  src/engine/local/client.rs          ChatClient, completions_url, parse_completion     (Task 5)
  src/engine/local/history.rs         load, repair                                      (Task 6)
  src/runtime.rs                      register LocalEngine, Config.local_retry_delays,
                                      Runtime::test_endpoint                            (Task 8)
  examples/smoke.rs                   --engine / --endpoint                             (Task 9)
  tests/store_dots_test.rs            endpoint URL cases                                (Task 1)
  tests/local_tools_test.rs           tools                                             (Tasks 2-4)
  tests/llm/mod.rs                    scripted OpenAI-compatible mock server            (Task 5)
  tests/local_client_test.rs          client                                            (Task 5)
  tests/local_history_test.rs         history                                           (Task 6)
  tests/local_engine_test.rs          engine through the Runner                         (Task 7)
  tests/runtime_local_test.rs         Runtime: approvals, resume, chat, test_endpoint   (Task 8)
docs/verification/2026-10-09-plan2-smoke.md                                             (Task 9)
```

---

### Task 1: Dependencies and the endpoint URL rule

**Files:**
- Modify: `crates/dots-core/Cargo.toml`
- Modify: `crates/dots-core/src/model.rs` (validation block in `DotSpec::validate`, new fn above `MAX_TIMEOUT_SECS`)
- Test: `crates/dots-core/tests/store_dots_test.rs` (`invalid_specs_are_rejected`)

**Interfaces:**
- Produces: `model::check_endpoint_url(url: &str) -> Result<()>` — `Err(Error::Invalid)` unless `url` (trimmed) starts with `http://` followed by a host. `DotSpec::validate` calls it for `EngineKind::Local`. Task 8's `probe` calls it too.

- [ ] **Step 1: Add the dependencies**

In `crates/dots-core/Cargo.toml`, add to `[dependencies]` (keep alphabetical order):
```toml
globset = "0.4"
ignore = "0.4"
regex = "1"
```
and to `[dev-dependencies]`:
```toml
wiremock = "0.6"
```

Run: `cargo build -p dots-core`
Expected: builds (the new crates download and compile; nothing uses them yet).

- [ ] **Step 2: Write the failing test**

In `crates/dots-core/tests/store_dots_test.rs`, in `invalid_specs_are_rejected`, add two specs after `zero_turns` and list them in the loop:
```rust
    let mut local_https = spec("https");
    local_https.engine = EngineKind::Local;
    local_https.endpoint_url = Some("https://example.com".into());
    let mut local_no_host = spec("nohost");
    local_no_host.engine = EngineKind::Local;
    local_no_host.endpoint_url = Some("http:///v1".into());
    for s in [
        bad_name,
        local_no_endpoint,
        local_https,
        local_no_host,
        five_field_cron,
        reserved_mcp,
        zero_turns,
    ] {
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cargo test -p dots-core --test store_dots_test invalid_specs_are_rejected`
Expected: FAIL — panic `called Result::unwrap_err() on an Ok value` (the `https` dot is accepted).

- [ ] **Step 4: Implement**

In `crates/dots-core/src/model.rs`, add above `pub const MAX_TIMEOUT_SECS`:
```rust
/// Local-engine endpoints are plain `http://` URLs (a local server); https is not supported.
pub fn check_endpoint_url(url: &str) -> Result<()> {
    let url = url.trim();
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        Error::Invalid(format!(
            "endpoint_url must start with http:// (https is not supported): {url}"
        ))
    })?;
    if rest.is_empty() || rest.starts_with('/') {
        return Err(Error::Invalid(format!("endpoint_url has no host: {url}")));
    }
    Ok(())
}
```
and in `DotSpec::validate` replace the `if self.engine == EngineKind::Local && ... { return invalid("the local engine requires endpoint_url"); }` block with:
```rust
        if self.engine == EngineKind::Local {
            match self.endpoint_url.as_deref() {
                Some(url) if !url.trim().is_empty() => check_endpoint_url(url)?,
                _ => return invalid("the local engine requires endpoint_url"),
            }
        }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p dots-core --test store_dots_test`
Expected: PASS (all tests in the file).

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/Cargo.toml Cargo.lock crates/dots-core/src/model.rs crates/dots-core/tests/store_dots_test.rs
git commit -m "feat(core): require an http:// endpoint for local dots and add local engine deps"
```

---

### Task 2: Tool calls — validation, workspace confinement, file tools

**Files:**
- Modify: `crates/dots-core/src/engine/mod.rs` (add `pub mod local;` after `pub mod claude;`)
- Create: `crates/dots-core/src/engine/local/mod.rs`
- Create: `crates/dots-core/src/engine/local/tools/mod.rs`
- Create: `crates/dots-core/src/engine/local/tools/fs.rs`
- Test: `crates/dots-core/tests/local_tools_test.rs`

**Interfaces:**
- Produces (module `dots_core::engine::local::tools`):
  - `TOOLS: &[(&str, &str)]` (local name, policy alias), `alias(name) -> Option<&'static str>`, `MAX_OUTPUT: usize` (30720), `MAX_ENTRIES: usize` (500).
  - `ToolOutput { output: String, is_error: bool }` with `ToolOutput::ok(..)` / `ToolOutput::err(..)`.
  - `PreparedCall { name: &'static str, alias: &'static str, input: Value }`.
  - `prepare(requested: &str, arguments: &Value, workspace: &Path) -> Result<PreparedCall, String>` — the `Err` text goes to the model as the tool result.
  - `run(call: &PreparedCall, workspace: &Path, cancel: &CancellationToken) -> ToolOutput` — never fails; errors become `is_error` outputs.
  - `confine(workspace: &Path, raw: &str) -> Result<PathBuf, String>`, `display(root: &Path, path: &Path) -> String`, `cap_head(String) -> String`, `cap_tail(String) -> String`, `schemas() -> Value` (OpenAI `tools` array, one entry per `TOOLS` row).
  - `pub(crate) str_arg(input: &Value, key: &str) -> &str` for the tool submodules.
- In this task `run` dispatches only the four file tools; Tasks 3 and 4 add `glob`/`grep` and `shell`.

- [ ] **Step 1: Write the failing tests**

`crates/dots-core/tests/local_tools_test.rs`:
```rust
use std::path::Path;

use dots_core::engine::local::tools::{self, ToolOutput, MAX_OUTPUT};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

fn workspace() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("README.md"), "hello\nworld\n").unwrap();
    std::fs::create_dir_all(dir.path().join("src/deep")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(dir.path().join("src/deep/lib.rs"), "pub fn Hello() {}\n").unwrap();
    dir
}

async fn run(ws: &Path, name: &str, args: Value) -> ToolOutput {
    let call = tools::prepare(name, &args, ws).unwrap_or_else(|e| panic!("prepare: {e}"));
    tools::run(&call, ws, &CancellationToken::new()).await
}

/// Creates a directory link at `link` pointing to `target` (a junction on Windows).
fn dir_link(target: &Path, link: &Path) {
    #[cfg(windows)]
    {
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J failed");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}

#[test]
fn confine_accepts_paths_inside_the_workspace() {
    let ws = workspace();
    let root = std::fs::canonicalize(ws.path()).unwrap();
    assert_eq!(
        tools::confine(ws.path(), "src/main.rs").unwrap(),
        root.join("src").join("main.rs")
    );
    assert_eq!(
        tools::confine(ws.path(), "./README.md").unwrap(),
        root.join("README.md")
    );
    let abs = ws.path().join("README.md");
    assert_eq!(
        tools::confine(ws.path(), abs.to_str().unwrap()).unwrap(),
        root.join("README.md")
    );
    assert_eq!(
        tools::confine(ws.path(), "new/dir/file.txt").unwrap(),
        root.join("new").join("dir").join("file.txt")
    );
    assert_eq!(tools::confine(ws.path(), ".").unwrap(), root);
}

#[test]
fn confine_rejects_escapes() {
    let ws = workspace();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "s").unwrap();
    let outside_file = outside.path().join("secret.txt");
    for raw in [
        "",
        "  ",
        "..",
        "../secret.txt",
        "src/../../secret.txt",
        "README.md:hidden",
        outside_file.to_str().unwrap(),
    ] {
        assert!(tools::confine(ws.path(), raw).is_err(), "accepted {raw:?}");
    }
    dir_link(outside.path(), &ws.path().join("escape"));
    assert!(tools::confine(ws.path(), "escape/secret.txt").is_err());
    assert!(tools::confine(ws.path(), "escape/new.txt").is_err());
}

#[test]
fn prepare_validates_calls_before_the_policy() {
    let ws = workspace();
    let p = ws.path();
    let call = tools::prepare("read_file", &json!("{\"file_path\":\"README.md\"}"), p).unwrap();
    assert_eq!(call.alias, "Read");
    assert_eq!(call.input, json!({"file_path": "README.md"}));
    let call = tools::prepare("shell", &json!({"command": "git status"}), p).unwrap();
    assert_eq!(call.alias, "Bash");
    let call = tools::prepare("list_dir", &json!(""), p).unwrap();
    assert_eq!(call.input, json!({}));
    let call = tools::prepare("glob", &json!({"pattern": "*.md", "path": ""}), p).unwrap();
    assert_eq!(call.input, json!({"pattern": "*.md"}));

    let err = tools::prepare("fly", &json!({}), p).unwrap_err();
    assert!(
        err.contains("unknown tool 'fly'") && err.contains("read_file"),
        "{err}"
    );
    let err = tools::prepare("read_file", &json!("{not json"), p).unwrap_err();
    assert!(err.contains("not valid JSON"), "{err}");
    let err = tools::prepare("read_file", &json!("[1]"), p).unwrap_err();
    assert!(err.contains("must be a JSON object"), "{err}");
    let err = tools::prepare(
        "edit_file",
        &json!({"file_path": "a", "old_string": "x"}),
        p,
    )
    .unwrap_err();
    assert!(err.contains("'new_string'"), "{err}");
    let err = tools::prepare("read_file", &json!({"file_path": "../x"}), p).unwrap_err();
    assert!(err.contains(".."), "{err}");
    assert_eq!(tools::alias("grep"), Some("Grep"));
    assert_eq!(
        tools::schemas().as_array().unwrap().len(),
        tools::TOOLS.len()
    );
}

#[tokio::test]
async fn read_file_returns_text_and_line_ranges() {
    let ws = workspace();
    let p = ws.path();
    assert_eq!(
        run(p, "read_file", json!({"file_path": "README.md"})).await,
        ToolOutput::ok("hello\nworld\n")
    );
    assert_eq!(
        run(
            p,
            "read_file",
            json!({"file_path": "README.md", "offset": 2, "limit": 1})
        )
        .await,
        ToolOutput::ok("world\n")
    );
    std::fs::write(p.join("bin.dat"), [0u8, 1, 2]).unwrap();
    let out = run(p, "read_file", json!({"file_path": "bin.dat"})).await;
    assert!(out.is_error && out.output.contains("binary"), "{out:?}");
    let out = run(p, "read_file", json!({"file_path": "src"})).await;
    assert!(out.is_error && out.output.contains("directory"), "{out:?}");
    let out = run(p, "read_file", json!({"file_path": "missing.txt"})).await;
    assert!(
        out.is_error && out.output.contains("cannot read"),
        "{out:?}"
    );
    std::fs::write(p.join("big.txt"), "y".repeat(MAX_OUTPUT * 2)).unwrap();
    let out = run(p, "read_file", json!({"file_path": "big.txt"})).await;
    assert!(out.output.len() < MAX_OUTPUT + 100 && out.output.contains("[truncated"));
}

#[tokio::test]
async fn write_and_edit_files() {
    let ws = workspace();
    let p = ws.path();
    let out = run(
        p,
        "write_file",
        json!({"file_path": "a/b/new.txt", "content": "one two"}),
    )
    .await;
    assert_eq!(out, ToolOutput::ok("wrote 7 bytes to a/b/new.txt"));
    assert_eq!(
        std::fs::read_to_string(p.join("a/b/new.txt")).unwrap(),
        "one two"
    );

    let edit = |file: &str, old: &str, new: &str| json!({"file_path": file, "old_string": old, "new_string": new});
    let out = run(p, "edit_file", edit("a/b/new.txt", "two", "2")).await;
    assert_eq!(out, ToolOutput::ok("edited a/b/new.txt"));
    assert_eq!(
        std::fs::read_to_string(p.join("a/b/new.txt")).unwrap(),
        "one 2"
    );
    let out = run(p, "edit_file", edit("a/b/new.txt", "zzz", "q")).await;
    assert!(out.is_error && out.output.contains("not found"), "{out:?}");
    std::fs::write(p.join("dup.txt"), "x x").unwrap();
    let out = run(p, "edit_file", edit("dup.txt", "x", "y")).await;
    assert!(out.is_error && out.output.contains("2 times"), "{out:?}");
    let out = run(p, "edit_file", edit("dup.txt", "", "y")).await;
    assert!(out.is_error, "{out:?}");
}

#[tokio::test]
async fn edit_file_matches_lf_text_in_crlf_files() {
    let ws = workspace();
    let p = ws.path();
    std::fs::write(p.join("crlf.txt"), "a\r\nb\r\nc\r\n").unwrap();
    let out = run(
        p,
        "edit_file",
        json!({"file_path": "crlf.txt", "old_string": "a\nb", "new_string": "A\nB"}),
    )
    .await;
    assert!(!out.is_error, "{out:?}");
    assert_eq!(
        std::fs::read_to_string(p.join("crlf.txt")).unwrap(),
        "A\r\nB\r\nc\r\n"
    );
}

#[tokio::test]
async fn list_dir_marks_directories_and_hides_git() {
    let ws = workspace();
    let p = ws.path();
    std::fs::create_dir(p.join(".git")).unwrap();
    assert_eq!(
        run(p, "list_dir", json!({})).await,
        ToolOutput::ok("README.md\nsrc/")
    );
    assert_eq!(
        run(p, "list_dir", json!({"path": "src"})).await,
        ToolOutput::ok("deep/\nmain.rs")
    );
    std::fs::create_dir(p.join("empty")).unwrap();
    assert_eq!(
        run(p, "list_dir", json!({"path": "empty"})).await,
        ToolOutput::ok("(empty directory)")
    );
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p dots-core --test local_tools_test`
Expected: FAIL to compile — unresolved import: `could not find local in engine`.

- [ ] **Step 3: Implement**

In `crates/dots-core/src/engine/mod.rs` add `pub mod local;` after `pub mod claude;`.

`crates/dots-core/src/engine/local/mod.rs`:
```rust
//! Local engine: a tool loop against an OpenAI-compatible chat endpoint (Ollama, LM Studio).

pub mod tools;
```

`crates/dots-core/src/engine/local/tools/mod.rs`:
```rust
//! Built-in tools of the local engine. Argument names match the Claude tools they alias, so
//! policy path specs, command rules, grants and approvals treat local and Claude calls alike.

use std::path::{Component, Path, PathBuf};

use serde_json::{json, Map, Value};
use tokio_util::sync::CancellationToken;

mod fs;

/// Tool output sent back to the model is capped at this many bytes.
pub const MAX_OUTPUT: usize = 30 * 1024;
/// Maximum entries listed by `list_dir` and `glob`.
pub const MAX_ENTRIES: usize = 500;

/// (local tool name, policy alias). The policy, grants and approvals only see the alias.
pub const TOOLS: &[(&str, &str)] = &[
    ("read_file", "Read"),
    ("write_file", "Write"),
    ("edit_file", "Edit"),
    ("list_dir", "LS"),
    ("glob", "Glob"),
    ("grep", "Grep"),
    ("shell", "Bash"),
];

#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutput {
    pub output: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: false,
        }
    }

    pub fn err(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            is_error: true,
        }
    }
}

/// A model tool call that passed `prepare` and is ready for the permission gate.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedCall {
    pub name: &'static str,
    pub alias: &'static str,
    pub input: Value,
}

pub fn alias(name: &str) -> Option<&'static str> {
    TOOLS.iter().find(|(n, _)| *n == name).map(|(_, a)| *a)
}

fn required(name: &str) -> &'static [&'static str] {
    match name {
        "read_file" => &["file_path"],
        "write_file" => &["file_path", "content"],
        "edit_file" => &["file_path", "old_string", "new_string"],
        "glob" | "grep" => &["pattern"],
        "shell" => &["command"],
        _ => &[],
    }
}

/// Checks a model tool call before the policy sees it: a known tool, arguments that form a
/// JSON object (sent as a JSON string, as an object, or empty), the required string arguments,
/// and every path inside the workspace. An empty optional `path` is dropped. The error text is
/// returned to the model as the tool result.
pub fn prepare(
    requested: &str,
    arguments: &Value,
    workspace: &Path,
) -> Result<PreparedCall, String> {
    let Some(&(name, alias)) = TOOLS.iter().find(|(n, _)| *n == requested) else {
        let names: Vec<&str> = TOOLS.iter().map(|(n, _)| *n).collect();
        return Err(format!(
            "unknown tool '{requested}'. Available tools: {}",
            names.join(", ")
        ));
    };
    let mut input: Map<String, Value> = match arguments {
        Value::Object(map) => map.clone(),
        Value::Null => Map::new(),
        Value::String(s) if s.trim().is_empty() => Map::new(),
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(Value::Object(map)) => map,
            Ok(_) => return Err(format!("arguments for {name} must be a JSON object")),
            Err(e) => {
                return Err(format!(
                    "arguments for {name} are not valid JSON ({e}); send a JSON object"
                ))
            }
        },
        _ => return Err(format!("arguments for {name} must be a JSON object")),
    };
    if input
        .get("path")
        .and_then(Value::as_str)
        .is_some_and(|p| p.trim().is_empty())
    {
        input.remove("path");
    }
    for key in required(name) {
        if !input.get(*key).is_some_and(Value::is_string) {
            return Err(format!("{name} requires the string argument '{key}'"));
        }
    }
    for key in ["file_path", "path"] {
        if let Some(raw) = input.get(key).and_then(Value::as_str) {
            confine(workspace, raw)?;
        }
    }
    Ok(PreparedCall {
        name,
        alias,
        input: Value::Object(input),
    })
}

/// Runs an allowed call. Every failure becomes an error result for the model.
pub async fn run(call: &PreparedCall, workspace: &Path, cancel: &CancellationToken) -> ToolOutput {
    let root = match std::fs::canonicalize(workspace) {
        Ok(r) => r,
        Err(e) => return ToolOutput::err(format!("workspace is not accessible: {e}")),
    };
    let input = &call.input;
    match call.name {
        "read_file" => fs::read_file(&root, input).await,
        "write_file" => fs::write_file(&root, input).await,
        "edit_file" => fs::edit_file(&root, input).await,
        "list_dir" => fs::list_dir(&root, input).await,
        other => ToolOutput::err(format!("unknown tool '{other}'")),
    }
}

/// Resolves `raw` (relative to the workspace, or absolute) to a path inside the workspace.
/// Symlinks and junctions are resolved through the longest existing ancestor, so a link that
/// points outside is rejected; `..`, NTFS stream names (`a.txt:s`) and empty paths are refused.
pub fn confine(workspace: &Path, raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("path must not be empty".into());
    }
    let given = Path::new(trimmed);
    for c in given.components() {
        match c {
            Component::ParentDir => return Err(format!("'..' is not allowed in paths: {raw}")),
            Component::Normal(n) if n.to_string_lossy().contains(':') => {
                return Err(format!("invalid path: {raw}"))
            }
            _ => {}
        }
    }
    let root = std::fs::canonicalize(workspace)
        .map_err(|e| format!("workspace is not accessible: {e}"))?;
    let mut existing = if given.is_absolute() {
        given.to_path_buf()
    } else {
        root.join(given)
    };
    let mut tail = Vec::new();
    let resolved = loop {
        match std::fs::canonicalize(&existing) {
            Ok(c) => break c,
            Err(_) => {
                let Some(name) = existing.file_name() else {
                    return Err(format!("invalid path: {raw}"));
                };
                tail.push(name.to_os_string());
                if !existing.pop() {
                    return Err(format!("invalid path: {raw}"));
                }
            }
        }
    };
    if !resolved.starts_with(&root) {
        return Err(format!("{raw} is outside the workspace"));
    }
    Ok(tail
        .into_iter()
        .rev()
        .fold(resolved, |p, name| p.join(name)))
}

/// `path` relative to the canonical workspace `root`, with `/` separators.
pub fn display(root: &Path, path: &Path) -> String {
    match path.strip_prefix(root) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// Keeps the first `MAX_OUTPUT` bytes.
pub fn cap_head(mut s: String) -> String {
    if s.len() <= MAX_OUTPUT {
        return s;
    }
    let mut cut = MAX_OUTPUT;
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    let dropped = s.len() - cut;
    s.truncate(cut);
    s.push_str(&format!("\n[truncated: {dropped} more bytes]"));
    s
}

/// Keeps the last `MAX_OUTPUT` bytes (the end of command output usually matters most).
pub fn cap_tail(s: String) -> String {
    if s.len() <= MAX_OUTPUT {
        return s;
    }
    let mut start = s.len() - MAX_OUTPUT;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    format!("[truncated: first {start} bytes omitted]\n{}", &s[start..])
}

pub(crate) fn str_arg<'a>(input: &'a Value, key: &str) -> &'a str {
    input.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {"type": "object", "properties": properties, "required": required}
        }
    })
}

#[cfg(windows)]
const SHELL_DESCRIPTION: &str = "Run a PowerShell command in the workspace directory and return \
    its output. Default timeout 120 seconds, maximum 600.";
#[cfg(not(windows))]
const SHELL_DESCRIPTION: &str = "Run a sh command in the workspace directory and return its \
    output. Default timeout 120 seconds, maximum 600.";

/// OpenAI `tools` array for every built-in tool.
pub fn schemas() -> Value {
    let s = |d: &str| json!({"type": "string", "description": d});
    let path = s("File path relative to the workspace root");
    Value::Array(vec![
        tool(
            "read_file",
            "Read a text file in the workspace.",
            json!({
                "file_path": path,
                "offset": {"type": "integer", "description": "First line to return, 1-based"},
                "limit": {"type": "integer", "description": "Maximum number of lines to return"}
            }),
            &["file_path"],
        ),
        tool(
            "write_file",
            "Create or overwrite a file with the given content.",
            json!({"file_path": path, "content": s("The complete new file content")}),
            &["file_path", "content"],
        ),
        tool(
            "edit_file",
            "Replace one exact, unique occurrence of old_string with new_string in a file.",
            json!({
                "file_path": path,
                "old_string": s("Exact text to replace; must occur exactly once"),
                "new_string": s("Replacement text")
            }),
            &["file_path", "old_string", "new_string"],
        ),
        tool(
            "list_dir",
            "List a directory. Directory names end with /.",
            json!({"path": s("Directory relative to the workspace root; default: the root")}),
            &[],
        ),
        tool(
            "glob",
            "Find files whose path matches a glob pattern such as **/*.rs.",
            json!({
                "pattern": s("Glob pattern, matched against paths relative to `path`"),
                "path": s("Directory to search; default: the workspace root")
            }),
            &["pattern"],
        ),
        tool(
            "grep",
            "Search file contents with a regular expression. Returns path:line: text lines.",
            json!({
                "pattern": s("Regular expression"),
                "path": s("File or directory to search; default: the workspace root"),
                "glob": s("Only search files matching this glob, e.g. *.rs"),
                "ignore_case": {"type": "boolean", "description": "Case-insensitive search"}
            }),
            &["pattern"],
        ),
        tool(
            "shell",
            SHELL_DESCRIPTION,
            json!({
                "command": s("The command to run"),
                "timeout": {"type": "integer", "description": "Timeout in seconds (1-600)"}
            }),
            &["command"],
        ),
    ])
}
```

`crates/dots-core/src/engine/local/tools/fs.rs`:
```rust
use std::path::Path;

use serde_json::Value;

use super::{cap_head, confine, display, str_arg, ToolOutput, MAX_ENTRIES};

/// Files whose first 8 KB contain a NUL byte are treated as binary and not returned.
fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

pub(super) async fn read_file(root: &Path, input: &Value) -> ToolOutput {
    let raw = str_arg(input, "file_path");
    let path = match confine(root, raw) {
        Ok(p) => p,
        Err(e) => return ToolOutput::err(e),
    };
    if path.is_dir() {
        return ToolOutput::err(format!("{raw} is a directory; use list_dir"));
    }
    let bytes = match tokio::fs::read(&path).await {
        Ok(b) => b,
        Err(e) => return ToolOutput::err(format!("cannot read {raw}: {e}")),
    };
    if is_binary(&bytes) {
        return ToolOutput::err(format!("{raw} is a binary file ({} bytes)", bytes.len()));
    }
    let text = String::from_utf8_lossy(&bytes);
    let offset = input
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let limit = input
        .get("limit")
        .and_then(Value::as_u64)
        .map(|n| n as usize);
    let lines = text.split_inclusive('\n').skip(offset - 1);
    let selected: String = match limit {
        Some(n) => lines.take(n).collect(),
        None => lines.collect(),
    };
    if selected.is_empty() {
        return ToolOutput::ok("(no content)");
    }
    ToolOutput::ok(cap_head(selected))
}

pub(super) async fn write_file(root: &Path, input: &Value) -> ToolOutput {
    let raw = str_arg(input, "file_path");
    let content = str_arg(input, "content");
    let path = match confine(root, raw) {
        Ok(p) => p,
        Err(e) => return ToolOutput::err(e),
    };
    if path.is_dir() {
        return ToolOutput::err(format!("{raw} is a directory"));
    }
    if let Some(parent) = path.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            return ToolOutput::err(format!("cannot create the directory for {raw}: {e}"));
        }
    }
    match tokio::fs::write(&path, content).await {
        Ok(()) => ToolOutput::ok(format!(
            "wrote {} bytes to {}",
            content.len(),
            display(root, &path)
        )),
        Err(e) => ToolOutput::err(format!("cannot write {raw}: {e}")),
    }
}

pub(super) async fn edit_file(root: &Path, input: &Value) -> ToolOutput {
    let raw = str_arg(input, "file_path");
    let mut old = str_arg(input, "old_string").to_string();
    let mut new = str_arg(input, "new_string").to_string();
    if old.is_empty() {
        return ToolOutput::err("old_string must not be empty; use write_file to create a file");
    }
    let path = match confine(root, raw) {
        Ok(p) => p,
        Err(e) => return ToolOutput::err(e),
    };
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(t) => t,
        Err(e) => return ToolOutput::err(format!("cannot read {raw}: {e}")),
    };
    // Windows checkouts often use CRLF while models write LF: retry with CRLF line endings.
    if !text.contains(&old) && text.contains("\r\n") && old.contains('\n') && !old.contains("\r\n")
    {
        old = old.replace('\n', "\r\n");
        new = new.replace("\r\n", "\n").replace('\n', "\r\n");
    }
    match text.matches(old.as_str()).count() {
        0 => return ToolOutput::err(format!("old_string was not found in {raw}")),
        1 => {}
        n => {
            return ToolOutput::err(format!(
            "old_string occurs {n} times in {raw}; include more surrounding text so it is unique"
        ))
        }
    }
    let updated = text.replacen(old.as_str(), &new, 1);
    match tokio::fs::write(&path, updated).await {
        Ok(()) => ToolOutput::ok(format!("edited {}", display(root, &path))),
        Err(e) => ToolOutput::err(format!("cannot write {raw}: {e}")),
    }
}

pub(super) async fn list_dir(root: &Path, input: &Value) -> ToolOutput {
    let raw = input.get("path").and_then(Value::as_str).unwrap_or(".");
    let dir = match confine(root, raw) {
        Ok(p) => p,
        Err(e) => return ToolOutput::err(e),
    };
    let raw = raw.to_string();
    let listed = tokio::task::spawn_blocking(move || {
        let entries = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) => return ToolOutput::err(format!("cannot list {raw}: {e}")),
        };
        let mut names: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name() != ".git")
            .map(|e| {
                let mut name = e.file_name().to_string_lossy().into_owned();
                if e.file_type().is_ok_and(|t| t.is_dir()) {
                    name.push('/');
                }
                name
            })
            .collect();
        if names.is_empty() {
            return ToolOutput::ok("(empty directory)");
        }
        names.sort();
        let total = names.len();
        names.truncate(MAX_ENTRIES);
        let mut out = names.join("\n");
        if total > MAX_ENTRIES {
            out.push_str(&format!("\n[{} more entries]", total - MAX_ENTRIES));
        }
        ToolOutput::ok(out)
    })
    .await;
    listed.unwrap_or_else(|e| ToolOutput::err(format!("list_dir failed: {e}")))
}
```

Notes for the implementer:
- `confine` refuses `..` lexically instead of resolving it, and walks up to the longest existing ancestor before `canonicalize`, so new files can be created while a junction/symlink that leaves the workspace is still caught. Both sides of `starts_with` are canonical (on Windows both carry the `\\?\` prefix).
- `prepare` passes the model's own arguments (minus an empty `path`) to the gate unchanged, so approvals show what the model asked for and `grant_key` matches the model's retry.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p dots-core --test local_tools_test`
Expected: PASS (7 tests). On Windows `confine_rejects_escapes` creates a junction with `mklink /J`, which needs no admin rights. rustc warns `unused variable: cancel` in `tools::run` until Task 4 adds the shell tool; leave it.

- [ ] **Step 5: Commit**

```bash
git add crates/dots-core/src/engine/mod.rs crates/dots-core/src/engine/local crates/dots-core/tests/local_tools_test.rs
git commit -m "feat(core): local tool calls with workspace confinement and file tools"
```

---

### Task 3: Search tools — glob and grep

**Files:**
- Create: `crates/dots-core/src/engine/local/tools/search.rs`
- Modify: `crates/dots-core/src/engine/local/tools/mod.rs` (module list, `run` match)
- Test: `crates/dots-core/tests/local_tools_test.rs` (append)

**Interfaces:**
- Consumes: Task 2's `confine`, `display`, `str_arg`, `cap_head`, `ToolOutput`, `MAX_ENTRIES`.
- Produces: `search::glob` / `search::grep` (private to `tools`), reachable through `tools::run` for `"glob"` and `"grep"`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/dots-core/tests/local_tools_test.rs`:
```rust
#[tokio::test]
async fn glob_matches_relative_paths_and_honours_gitignore() {
    let ws = workspace();
    let p = ws.path();
    assert_eq!(
        run(p, "glob", json!({"pattern": "**/*.rs"})).await,
        ToolOutput::ok("src/deep/lib.rs\nsrc/main.rs")
    );
    assert_eq!(
        run(p, "glob", json!({"pattern": "*.md"})).await,
        ToolOutput::ok("README.md")
    );
    assert_eq!(
        run(p, "glob", json!({"pattern": "*.rs", "path": "src"})).await,
        ToolOutput::ok("src/main.rs")
    );
    std::fs::write(p.join(".gitignore"), "target/\n").unwrap();
    std::fs::create_dir(p.join("target")).unwrap();
    std::fs::write(p.join("target/gen.rs"), "").unwrap();
    let out = run(p, "glob", json!({"pattern": "**/*.rs"})).await;
    assert!(!out.output.contains("target"), "{out:?}");
    assert_eq!(
        run(p, "glob", json!({"pattern": "**/*.zip"})).await,
        ToolOutput::ok("no files match **/*.zip")
    );
}

#[tokio::test]
async fn grep_reports_path_line_and_text() {
    let ws = workspace();
    let p = ws.path();
    assert_eq!(
        run(p, "grep", json!({"pattern": "fn \\w+"})).await,
        ToolOutput::ok("src/deep/lib.rs:1: pub fn Hello() {}\nsrc/main.rs:1: fn main() {}")
    );
    assert_eq!(
        run(
            p,
            "grep",
            json!({"pattern": "hello", "ignore_case": true, "glob": "*.rs"})
        )
        .await,
        ToolOutput::ok("src/deep/lib.rs:1: pub fn Hello() {}")
    );
    assert_eq!(
        run(p, "grep", json!({"pattern": "world", "path": "README.md"})).await,
        ToolOutput::ok("README.md:2: world")
    );
    let out = run(p, "grep", json!({"pattern": "("})).await;
    assert!(
        out.is_error && out.output.contains("invalid regular expression"),
        "{out:?}"
    );
    assert_eq!(
        run(p, "grep", json!({"pattern": "nothing-here"})).await,
        ToolOutput::ok("no matches for nothing-here")
    );
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p dots-core --test local_tools_test -- glob grep`
Expected: FAIL — the outputs are `unknown tool 'glob'` / `unknown tool 'grep'` errors, not the expected listings.

- [ ] **Step 3: Implement**

`crates/dots-core/src/engine/local/tools/search.rs`:
```rust
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{cap_head, confine, display, str_arg, ToolOutput, MAX_ENTRIES};

const MAX_MATCHES: usize = 200;
const MAX_LINE_CHARS: usize = 300;
/// Files larger than this are skipped by `grep`.
const MAX_GREP_FILE: u64 = 2 * 1024 * 1024;

/// Walks `base`, honouring `.gitignore` files and skipping `.git`.
fn walk(base: &Path) -> impl Iterator<Item = PathBuf> {
    ignore::WalkBuilder::new(base)
        .hidden(false)
        .require_git(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(|e| e.into_path())
}

fn base_dir(root: &Path, input: &Value) -> Result<PathBuf, String> {
    confine(
        root,
        input.get("path").and_then(Value::as_str).unwrap_or("."),
    )
}

pub(super) async fn glob(root: &Path, input: &Value) -> ToolOutput {
    let pattern = str_arg(input, "pattern").replace('\\', "/");
    let base = match base_dir(root, input) {
        Ok(b) => b,
        Err(e) => return ToolOutput::err(e),
    };
    let matcher = match globset::GlobBuilder::new(&pattern)
        .literal_separator(true)
        .build()
    {
        Ok(g) => g.compile_matcher(),
        Err(e) => return ToolOutput::err(format!("invalid glob pattern: {e}")),
    };
    let root = root.to_path_buf();
    let found = tokio::task::spawn_blocking(move || {
        let mut hits: Vec<String> = walk(&base)
            .filter(|p| matcher.is_match(p.strip_prefix(&base).unwrap_or(p)))
            .map(|p| display(&root, &p))
            .collect();
        if hits.is_empty() {
            return ToolOutput::ok(format!("no files match {pattern}"));
        }
        hits.sort();
        let total = hits.len();
        hits.truncate(MAX_ENTRIES);
        let mut out = hits.join("\n");
        if total > MAX_ENTRIES {
            out.push_str(&format!("\n[{} more files]", total - MAX_ENTRIES));
        }
        ToolOutput::ok(out)
    })
    .await;
    found.unwrap_or_else(|e| ToolOutput::err(format!("glob failed: {e}")))
}

pub(super) async fn grep(root: &Path, input: &Value) -> ToolOutput {
    let pattern = str_arg(input, "pattern").to_string();
    let ignore_case = input
        .get("ignore_case")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let re = match regex::RegexBuilder::new(&pattern)
        .case_insensitive(ignore_case)
        .build()
    {
        Ok(r) => r,
        Err(e) => return ToolOutput::err(format!("invalid regular expression: {e}")),
    };
    let filter = match input.get("glob").and_then(Value::as_str) {
        Some(g) if !g.trim().is_empty() => match globset::Glob::new(&g.replace('\\', "/")) {
            Ok(g) => Some(g.compile_matcher()),
            Err(e) => return ToolOutput::err(format!("invalid glob pattern: {e}")),
        },
        _ => None,
    };
    let base = match base_dir(root, input) {
        Ok(b) => b,
        Err(e) => return ToolOutput::err(e),
    };
    let root = root.to_path_buf();
    let found = tokio::task::spawn_blocking(move || {
        let mut hits = Vec::new();
        'files: for path in walk(&base) {
            let rel = display(&root, &path);
            if filter.as_ref().is_some_and(|f| !f.is_match(&rel)) {
                continue;
            }
            if std::fs::metadata(&path).map_or(true, |m| m.len() > MAX_GREP_FILE) {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if bytes.iter().take(8192).any(|b| *b == 0) {
                continue;
            }
            for (i, line) in String::from_utf8_lossy(&bytes).lines().enumerate() {
                if re.is_match(line) {
                    let line: String = line.trim_end().chars().take(MAX_LINE_CHARS).collect();
                    hits.push(format!("{rel}:{}: {line}", i + 1));
                    if hits.len() >= MAX_MATCHES {
                        hits.push(format!("[stopped after {MAX_MATCHES} matches]"));
                        break 'files;
                    }
                }
            }
        }
        if hits.is_empty() {
            return ToolOutput::ok(format!("no matches for {pattern}"));
        }
        ToolOutput::ok(cap_head(hits.join("\n")))
    })
    .await;
    found.unwrap_or_else(|e| ToolOutput::err(format!("grep failed: {e}")))
}
```

In `crates/dots-core/src/engine/local/tools/mod.rs` add `mod search;` after `mod fs;`, and in `run` add before the `other =>` arm:
```rust
        "glob" => search::glob(&root, input).await,
        "grep" => search::grep(&root, input).await,
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p dots-core --test local_tools_test`
Expected: PASS (9 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/dots-core/src/engine/local/tools crates/dots-core/tests/local_tools_test.rs
git commit -m "feat(core): glob and grep tools for the local engine"
```

---

### Task 4: Shell tool

**Files:**
- Modify: `crates/dots-core/src/proc.rs` (add `read_tail`)
- Modify: `crates/dots-core/src/engine/claude/mod.rs` (remove its private `read_tail`, call `crate::proc::read_tail`)
- Create: `crates/dots-core/src/engine/local/tools/shell.rs`
- Modify: `crates/dots-core/src/engine/local/tools/mod.rs` (module list, `run` match)
- Test: `crates/dots-core/tests/local_tools_test.rs` (append)

**Interfaces:**
- Produces: `proc::read_tail(r: impl AsyncRead + Unpin, max: usize) -> String` (moved, unchanged); `shell::shell` reachable through `tools::run` for `"shell"`.
- Behavior: Windows runs `powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand <base64 UTF-16LE>` with UTF-8 output; other OSes run `sh -c`. cwd = workspace, stdin closed, hidden window. Exit 0 → ok output (or `(no output)`); nonzero → `exit code N\n<output>` as an error; timeout → `timed out after Ns and was stopped`; cancel → `cancelled`. The process tree is killed on timeout and cancel. stdout then stderr, last 30 KB kept.

- [ ] **Step 1: Write the failing tests**

In `crates/dots-core/tests/local_tools_test.rs` add `use std::time::{Duration, Instant};` to the imports and append:
```rust
#[cfg(windows)]
mod cmds {
    pub const ECHO: &str = "Write-Output 'hi there'";
    pub const FAIL: &str = "cmd /c exit 3";
    pub const SLEEP: &str = "Start-Sleep -Seconds 30";
    pub const BIG: &str = "'x' * 100000";
}
#[cfg(not(windows))]
mod cmds {
    pub const ECHO: &str = "echo 'hi there'";
    pub const FAIL: &str = "exit 3";
    pub const SLEEP: &str = "sleep 30";
    pub const BIG: &str = "head -c 100000 /dev/zero | tr '\\0' x";
}

#[tokio::test]
async fn shell_runs_in_the_workspace_and_reports_exit_codes() {
    let ws = workspace();
    let p = ws.path();
    let out = run(p, "shell", json!({"command": cmds::ECHO})).await;
    assert_eq!(out.output.trim(), "hi there");
    assert!(!out.is_error);
    let out = run(p, "shell", json!({"command": cmds::FAIL})).await;
    assert!(
        out.is_error && out.output.starts_with("exit code 3"),
        "{out:?}"
    );
    let out = run(p, "shell", json!({"command": cmds::BIG})).await;
    assert!(out.output.len() < MAX_OUTPUT + 100 && out.output.starts_with("[truncated"));
}

#[tokio::test]
async fn shell_stops_at_its_timeout_and_on_cancel() {
    let ws = workspace();
    let p = ws.path();
    let started = Instant::now();
    let out = run(p, "shell", json!({"command": cmds::SLEEP, "timeout": 1})).await;
    assert!(
        out.is_error && out.output.contains("timed out after 1s"),
        "{out:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(15));

    let call = tools::prepare("shell", &json!({"command": cmds::SLEEP}), p).unwrap();
    let cancel = CancellationToken::new();
    let c = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        c.cancel();
    });
    let started = Instant::now();
    let out = tools::run(&call, p, &cancel).await;
    assert_eq!(out, ToolOutput::err("cancelled"));
    assert!(started.elapsed() < Duration::from_secs(15));
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p dots-core --test local_tools_test -- shell`
Expected: FAIL — the outputs are `unknown tool 'shell'` errors.

- [ ] **Step 3: Move `read_tail` into `proc.rs`**

In `crates/dots-core/src/engine/claude/mod.rs` delete `async fn read_tail(...) { ... }`, change the import to `use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};`, and change the stderr task to:
```rust
            let mut stderr_task = tokio::spawn(crate::proc::read_tail(stderr, STDERR_TAIL));
```
In `crates/dots-core/src/proc.rs` add `use tokio::io::{AsyncRead, AsyncReadExt};` at the top and append:
```rust
/// Reads `r` to the end and returns its last `max` bytes, lossily decoded as UTF-8.
pub async fn read_tail(mut r: impl AsyncRead + Unpin, max: usize) -> String {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match r.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.len() > max {
                    let excess = buf.len() - max;
                    buf.drain(..excess);
                }
            }
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}
```

Run: `cargo test -p dots-core --test claude_engine_test`
Expected: PASS (unchanged behavior).

- [ ] **Step 4: Implement the tool**

`crates/dots-core/src/engine/local/tools/shell.rs`:
```rust
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use super::{cap_tail, str_arg, ToolOutput, MAX_OUTPUT};

const DEFAULT_TIMEOUT_SECS: u64 = 120;
const MAX_TIMEOUT_SECS: u64 = 600;

/// The shell process for `script`. On Windows the script goes through `-EncodedCommand`, which
/// avoids every quoting problem of passing it as an argument, and prints UTF-8. The trailer
/// turns a failed last command into a nonzero exit code (native exit code when there is one).
#[cfg(windows)]
fn command_for(script: &str) -> Command {
    use base64::Engine as _;
    let full = format!(
        "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8\n\
         $ProgressPreference = 'SilentlyContinue'\n\
         {script}\n\
         if ($?) {{ exit 0 }} elseif ($LASTEXITCODE) {{ exit $LASTEXITCODE }} else {{ exit 1 }}"
    );
    let utf16: Vec<u8> = full.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let mut cmd = Command::new("powershell.exe");
    cmd.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-EncodedCommand",
    ])
    .arg(base64::engine::general_purpose::STANDARD.encode(utf16));
    cmd
}

#[cfg(not(windows))]
fn command_for(script: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(script);
    cmd
}

enum End {
    Exited(Option<i32>),
    TimedOut,
    Cancelled,
}

pub(super) async fn shell(root: &Path, input: &Value, cancel: &CancellationToken) -> ToolOutput {
    let script = str_arg(input, "command");
    let secs = input
        .get("timeout")
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
        .clamp(1, MAX_TIMEOUT_SECS);
    let mut cmd = command_for(script);
    cmd.current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    crate::proc::hide_window(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ToolOutput::err(format!("failed to start the shell: {e}")),
    };
    let pid = child.id();
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let out_task = tokio::spawn(crate::proc::read_tail(stdout, 2 * MAX_OUTPUT));
    let err_task = tokio::spawn(crate::proc::read_tail(stderr, 2 * MAX_OUTPUT));

    let end = tokio::select! {
        s = child.wait() => End::Exited(s.ok().and_then(|s| s.code())),
        _ = tokio::time::sleep(Duration::from_secs(secs)) => End::TimedOut,
        _ = cancel.cancelled() => End::Cancelled,
    };
    if !matches!(end, End::Exited(_)) {
        if let Some(pid) = pid {
            crate::proc::kill_tree(pid).await;
        }
        let _ = child.start_kill();
        let _ = child.wait().await;
    }
    // A grandchild that survived the kill may hold the pipes open; never wait on it for long.
    let cap = Duration::from_secs(5);
    let out = tokio::time::timeout(cap, out_task)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let err = tokio::time::timeout(cap, err_task)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let mut text = out;
    if !err.trim().is_empty() {
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&err);
    }
    let text = cap_tail(text);
    match end {
        End::Exited(Some(0)) if text.trim().is_empty() => ToolOutput::ok("(no output)"),
        End::Exited(Some(0)) => ToolOutput::ok(text),
        End::Exited(code) => ToolOutput::err(format!(
            "exit code {}\n{text}",
            code.map_or_else(|| "unknown".to_string(), |c| c.to_string())
        )),
        End::TimedOut => {
            ToolOutput::err(format!("timed out after {secs}s and was stopped\n{text}"))
        }
        End::Cancelled => ToolOutput::err("cancelled"),
    }
}
```

In `crates/dots-core/src/engine/local/tools/mod.rs` add `mod shell;` after `mod search;`, and in `run` add before the `other =>` arm:
```rust
        "shell" => shell::shell(&root, input, cancel).await,
```

Why the trailer line in the PowerShell script: `powershell -EncodedCommand` exits 0 or 1 regardless of a failing native command's code. `if ($?) { exit 0 } elseif ($LASTEXITCODE) { exit $LASTEXITCODE } else { exit 1 }` reports the native exit code when the last command failed.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p dots-core --test local_tools_test`
Expected: PASS (11 tests; the timeout/cancel test takes a few seconds).

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/proc.rs crates/dots-core/src/engine/claude/mod.rs crates/dots-core/src/engine/local/tools crates/dots-core/tests/local_tools_test.rs
git commit -m "feat(core): shell tool for the local engine"
```

---

### Task 5: Chat completions client

**Files:**
- Create: `crates/dots-core/src/engine/local/client.rs`
- Modify: `crates/dots-core/src/engine/local/mod.rs` (add `pub mod client;`)
- Create: `crates/dots-core/tests/llm/mod.rs` (shared mock endpoint for Tasks 5, 7, 8)
- Test: `crates/dots-core/tests/local_client_test.rs`

**Interfaces:**
- Produces (module `dots_core::engine::local::client`):
  - `DEFAULT_RETRY_DELAYS: [Duration; 2]` (1 s, 2 s).
  - `completions_url(endpoint: &str) -> String`.
  - `ToolCallRequest { id: String, name: String, arguments: Value }`.
  - `Completion { message: Value, content: String, tool_calls: Vec<ToolCallRequest>, tokens_in: u64, tokens_out: u64 }` — `message` is the assistant message ready to append (string `arguments`, an id on every call).
  - `ChatClient::new(retry_delays: Vec<Duration>)`; `ChatClient::complete(&self, endpoint: &str, body: &Value, turn: u32, cancel: &CancellationToken) -> Result<Option<Completion>>` — `Ok(None)` when cancelled; 5xx / unreachable retried once per delay; 4xx and non-JSON fail at once with the body (≤ 500 chars) in the message.
  - `parse_completion(v: &Value, turn: u32) -> Result<Completion>` — missing ids become `call_<turn>_<index>`.
- Test helper `tests/llm/mod.rs`: `Llm::start(Vec<ResponseTemplate>)`, `.endpoint()`, `.requests()`, `.wait_requests(n, secs)`; builders `completion`, `text`, `call`, `calls`; readers `messages(req)`, `tool_result(req, id)`. An exhausted script answers 500.

- [ ] **Step 1: Write the mock endpoint and the failing tests**

`crates/dots-core/tests/llm/mod.rs`:
```rust
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
```

`crates/dots-core/tests/local_client_test.rs`:
```rust
mod llm;

use std::time::Duration;

use dots_core::engine::local::client::{completions_url, parse_completion, ChatClient};
use llm::{call, completion, text, Llm};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use wiremock::ResponseTemplate;

fn client() -> ChatClient {
    ChatClient::new(vec![Duration::ZERO, Duration::ZERO])
}

#[test]
fn completions_url_accepts_v1_and_trailing_slashes() {
    for endpoint in [
        "http://127.0.0.1:1234",
        "http://127.0.0.1:1234/",
        "http://127.0.0.1:1234/v1",
        " http://127.0.0.1:1234/v1/ ",
    ] {
        assert_eq!(
            completions_url(endpoint),
            "http://127.0.0.1:1234/v1/chat/completions"
        );
    }
}

#[test]
fn parse_completion_normalizes_tool_calls() {
    let reply = json!({
        "choices": [{"message": {
            "role": "assistant",
            "content": null,
            "tool_calls": [
                {"type": "function", "function": {"name": "list_dir", "arguments": {"path": "src"}}},
                {"id": "", "type": "function", "function": {"name": "glob"}}
            ]
        }}],
        "usage": {"prompt_tokens": 7, "completion_tokens": 3}
    });
    let c = parse_completion(&reply, 4).unwrap();
    assert_eq!(c.content, "");
    assert_eq!(c.tool_calls[0].id, "call_4_0");
    assert_eq!(c.tool_calls[0].arguments, json!({"path": "src"}));
    assert_eq!(c.tool_calls[1].id, "call_4_1");
    assert_eq!((c.tokens_in, c.tokens_out), (7, 3));
    assert_eq!(
        c.message,
        json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [
                {"id": "call_4_0", "type": "function",
                 "function": {"name": "list_dir", "arguments": "{\"path\":\"src\"}"}},
                {"id": "call_4_1", "type": "function",
                 "function": {"name": "glob", "arguments": "{}"}}
            ]
        })
    );
    assert!(parse_completion(&json!({"choices": []}), 1).is_err());
}

#[tokio::test]
async fn complete_posts_the_body_and_reads_the_reply() {
    let llm = Llm::start(vec![call("c1", "read_file", json!({"file_path": "a"}))]).await;
    let body = json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
    let c = client()
        .complete(
            &format!("{}/v1", llm.endpoint()),
            &body,
            1,
            &CancellationToken::new(),
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(c.tool_calls[0].name, "read_file");
    assert_eq!(c.tool_calls[0].arguments, json!("{\"file_path\":\"a\"}"));
    assert_eq!(llm.requests(), vec![body]);
}

#[tokio::test]
async fn server_errors_are_retried_then_reported() {
    let llm = Llm::start(vec![
        ResponseTemplate::new(503).set_body_string("loading"),
        text("ok"),
    ])
    .await;
    let c = client()
        .complete(&llm.endpoint(), &json!({}), 1, &CancellationToken::new())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(c.content, "ok");

    let llm = Llm::start(vec![
        ResponseTemplate::new(500).set_body_string("boom"),
        ResponseTemplate::new(500).set_body_string("boom"),
        ResponseTemplate::new(500).set_body_string("boom"),
    ])
    .await;
    let err = client()
        .complete(&llm.endpoint(), &json!({}), 1, &CancellationToken::new())
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("500") && err.contains("boom") && err.contains("3 attempts"),
        "{err}"
    );
    assert_eq!(llm.requests().len(), 3);
}

#[tokio::test]
async fn client_errors_and_bad_bodies_fail_at_once() {
    let llm = Llm::start(vec![
        ResponseTemplate::new(404).set_body_json(json!({"error": "model 'x' not loaded"}))
    ])
    .await;
    let err = client()
        .complete(&llm.endpoint(), &json!({}), 1, &CancellationToken::new())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("404") && err.contains("not loaded"), "{err}");
    assert_eq!(llm.requests().len(), 1);

    let llm = Llm::start(vec![ResponseTemplate::new(200).set_body_string("<html>")]).await;
    let err = client()
        .complete(&llm.endpoint(), &json!({}), 1, &CancellationToken::new())
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("invalid JSON"), "{err}");
}

#[tokio::test]
async fn unreachable_endpoint_is_reported_after_retries() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let err = client()
        .complete(
            &format!("http://127.0.0.1:{port}"),
            &json!({}),
            1,
            &CancellationToken::new(),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("unreachable") && err.contains("3 attempts"),
        "{err}"
    );
}

#[tokio::test]
async fn cancel_interrupts_a_slow_request() {
    let slow = completion(json!({"role": "assistant", "content": "late"}))
        .set_delay(Duration::from_secs(30));
    let llm = Llm::start(vec![slow]).await;
    let cancel = CancellationToken::new();
    let c = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        c.cancel();
    });
    let started = std::time::Instant::now();
    let out = client()
        .complete(&llm.endpoint(), &json!({}), 1, &cancel)
        .await
        .unwrap();
    assert!(out.is_none());
    assert!(started.elapsed() < Duration::from_secs(5));
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p dots-core --test local_client_test`
Expected: FAIL to compile — unresolved import: `could not find client in local`.

- [ ] **Step 3: Implement**

`crates/dots-core/src/engine/local/client.rs`:
```rust
//! Minimal non-streaming client for an OpenAI-compatible `/v1/chat/completions` endpoint.

use std::time::Duration;

use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::{Error, Result};

/// Delays before the second and third attempt after an unreachable endpoint or a 5xx reply.
pub const DEFAULT_RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(2)];

/// `{endpoint}/v1/chat/completions`. A trailing `/` or `/v1` on the endpoint is accepted, so
/// both `http://127.0.0.1:1234` and `http://127.0.0.1:1234/v1` work.
pub fn completions_url(endpoint: &str) -> String {
    let base = endpoint.trim().trim_end_matches('/');
    let base = base.strip_suffix("/v1").unwrap_or(base);
    format!("{base}/v1/chat/completions")
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallRequest {
    pub id: String,
    pub name: String,
    /// As sent by the model: usually a JSON string, sometimes an object.
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Completion {
    /// The assistant message in request form (string `arguments`, an id on every tool call),
    /// ready to append to the conversation.
    pub message: Value,
    pub content: String,
    pub tool_calls: Vec<ToolCallRequest>,
    pub tokens_in: u64,
    pub tokens_out: u64,
}

enum SendError {
    Retryable(String),
    Fatal(String),
}

fn snippet(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() > 500 {
        format!("{}…", t.chars().take(500).collect::<String>())
    } else {
        t.to_string()
    }
}

#[derive(Clone)]
pub struct ChatClient {
    http: reqwest::Client,
    retry_delays: Vec<Duration>,
}

impl ChatClient {
    pub fn new(retry_delays: Vec<Duration>) -> Self {
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("building the HTTP client never fails without TLS options");
        Self { http, retry_delays }
    }

    /// Sends one chat completion request. Unreachable endpoints and 5xx replies are retried
    /// after each of the retry delays; 4xx replies and malformed bodies fail at once with the
    /// endpoint's message. Tool calls without an id get `call_<turn>_<index>`.
    /// `Ok(None)` means `cancel` fired first.
    pub async fn complete(
        &self,
        endpoint: &str,
        body: &Value,
        turn: u32,
        cancel: &CancellationToken,
    ) -> Result<Option<Completion>> {
        let url = completions_url(endpoint);
        let mut attempt = 0;
        loop {
            let sent = tokio::select! {
                r = self.send(&url, body) => r,
                _ = cancel.cancelled() => return Ok(None),
            };
            let error = match sent {
                Ok(v) => return parse_completion(&v, turn).map(Some),
                Err(SendError::Fatal(e)) => return Err(Error::Other(e)),
                Err(SendError::Retryable(e)) => e,
            };
            let Some(delay) = self.retry_delays.get(attempt) else {
                return Err(Error::Other(format!(
                    "{error} (gave up after {} attempts)",
                    attempt + 1
                )));
            };
            attempt += 1;
            tokio::select! {
                _ = tokio::time::sleep(*delay) => {}
                _ = cancel.cancelled() => return Ok(None),
            }
        }
    }

    async fn send(&self, url: &str, body: &Value) -> std::result::Result<Value, SendError> {
        let resp =
            self.http.post(url).json(body).send().await.map_err(|e| {
                SendError::Retryable(format!("local endpoint {url} unreachable: {e}"))
            })?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| {
            SendError::Retryable(format!("reading the reply from {url} failed: {e}"))
        })?;
        if status.is_server_error() {
            return Err(SendError::Retryable(format!(
                "local endpoint returned {status}: {}",
                snippet(&text)
            )));
        }
        if !status.is_success() {
            return Err(SendError::Fatal(format!(
                "local endpoint returned {status}: {}",
                snippet(&text)
            )));
        }
        serde_json::from_str(&text).map_err(|e| {
            SendError::Fatal(format!(
                "local endpoint returned invalid JSON ({e}): {}",
                snippet(&text)
            ))
        })
    }
}

/// Reads `choices[0].message` of a chat completion reply.
pub fn parse_completion(v: &Value, turn: u32) -> Result<Completion> {
    let msg = v.pointer("/choices/0/message").ok_or_else(|| {
        Error::Other(format!(
            "local endpoint reply has no choices[0].message: {}",
            snippet(&v.to_string())
        ))
    })?;
    let content = match msg.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    };
    let tool_calls: Vec<ToolCallRequest> = msg
        .get("tool_calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .map(|(i, tc)| ToolCallRequest {
            id: tc
                .get("id")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map_or_else(|| format!("call_{turn}_{i}"), String::from),
            name: tc
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            arguments: tc
                .pointer("/function/arguments")
                .cloned()
                .unwrap_or(Value::Null),
        })
        .collect();
    let mut message = json!({"role": "assistant", "content": content});
    if !tool_calls.is_empty() {
        let calls: Vec<Value> = tool_calls
            .iter()
            .map(|c| {
                let arguments = match &c.arguments {
                    Value::String(s) => s.clone(),
                    Value::Null => "{}".to_string(),
                    other => other.to_string(),
                };
                json!({
                    "id": c.id,
                    "type": "function",
                    "function": {"name": c.name, "arguments": arguments}
                })
            })
            .collect();
        message["tool_calls"] = Value::Array(calls);
    }
    let usage = |k: &str| {
        v.get("usage")
            .and_then(|u| u.get(k))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    Ok(Completion {
        message,
        content,
        tool_calls,
        tokens_in: usage("prompt_tokens"),
        tokens_out: usage("completion_tokens"),
    })
}
```

In `crates/dots-core/src/engine/local/mod.rs` add `pub mod client;` above `pub mod tools;`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p dots-core --test local_client_test`
Expected: PASS (7 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/dots-core/src/engine/local crates/dots-core/tests/llm crates/dots-core/tests/local_client_test.rs
git commit -m "feat(core): OpenAI-compatible chat client with retries for the local engine"
```

---

### Task 6: Conversation history and repair

**Files:**
- Create: `crates/dots-core/src/engine/local/history.rs`
- Modify: `crates/dots-core/src/engine/local/mod.rs` (add `pub mod history;`)
- Test: `crates/dots-core/tests/local_history_test.rs`

**Interfaces:**
- Consumes: `Store::get_run`, `Store::list_events` (Plan 1). Event rows of kind `message` with `data = {"kind": "message", "message": <chat message>}` (written by Task 7).
- Produces: `history::INTERRUPTED: &str`; `history::repair(messages: Vec<Value>) -> Vec<Value>`; `history::load(store: &Store, run_id: &str) -> Result<Vec<Value>>` — the repaired conversation of `run_id`'s ancestors, oldest first, following `parent_run_id` (sibling branches excluded; `run_id`'s own events excluded).

- [ ] **Step 1: Write the failing tests**

`crates/dots-core/tests/local_history_test.rs`:
```rust
mod common;

use common::{spec, temp_store};
use dots_core::engine::local::history::{self, INTERRUPTED};
use dots_core::model::{EngineKind, NewRun, TriggerKind};
use serde_json::{json, Value};

fn user(t: &str) -> Value {
    json!({"role": "user", "content": t})
}

fn assistant(t: &str) -> Value {
    json!({"role": "assistant", "content": t})
}

fn calling(ids: &[&str]) -> Value {
    let calls: Vec<Value> = ids
        .iter()
        .map(|id| {
            json!({"id": id, "type": "function",
                   "function": {"name": "list_dir", "arguments": "{}"}})
        })
        .collect();
    json!({"role": "assistant", "content": "", "tool_calls": calls})
}

fn tool(id: &str, out: &str) -> Value {
    json!({"role": "tool", "tool_call_id": id, "content": out})
}

#[test]
fn repair_keeps_a_valid_conversation_unchanged() {
    let conv = vec![
        json!({"role": "system", "content": "s"}),
        user("u"),
        calling(&["a", "b"]),
        tool("a", "1"),
        tool("b", "2"),
        assistant("done"),
    ];
    assert_eq!(history::repair(conv.clone()), conv);
}

#[test]
fn repair_answers_unanswered_tool_calls() {
    let conv = vec![
        user("u"),
        calling(&["a", "b"]),
        tool("a", "1"),
        user("next"),
    ];
    assert_eq!(
        history::repair(conv),
        vec![
            user("u"),
            calling(&["a", "b"]),
            tool("a", "1"),
            tool("b", INTERRUPTED),
            user("next"),
        ]
    );
    let trailing = vec![user("u"), calling(&["a"])];
    assert_eq!(
        history::repair(trailing),
        vec![user("u"), calling(&["a"]), tool("a", INTERRUPTED)]
    );
}

#[test]
fn repair_drops_orphan_results_and_merges_user_messages() {
    let conv = vec![
        user("first"),
        tool("ghost", "x"),
        user("second"),
        assistant("ok"),
    ];
    assert_eq!(
        history::repair(conv),
        vec![user("first\n\nsecond"), assistant("ok")]
    );
}

#[tokio::test]
async fn load_follows_the_parent_chain_in_order() {
    let (_d, store) = temp_store().await;
    let mut s = spec("hist");
    s.engine = EngineKind::Local;
    s.endpoint_url = Some("http://127.0.0.1:1".into());
    let dot = store.create_dot(&s).await.unwrap();
    let root = store
        .create_run(&NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap();
    let child_of = |parent: &str| {
        let mut new = NewRun::new(&dot.id, TriggerKind::Chat);
        new.parent_run_id = Some(parent.to_string());
        new.root_run_id = Some(root.id.clone());
        new
    };
    let child = store.create_run(&child_of(&root.id)).await.unwrap();
    let grandchild = store.create_run(&child_of(&child.id)).await.unwrap();
    // A sibling branch of the same root must not leak into the chain.
    let sibling = store.create_run(&child_of(&root.id)).await.unwrap();

    let add = |run: String, m: Value| {
        let store = store.clone();
        async move {
            store
                .append_event(&run, "message", &json!({"kind": "message", "message": m}))
                .await
                .unwrap();
        }
    };
    add(root.id.clone(), user("one")).await;
    store
        .append_event(
            &root.id,
            "assistant_text",
            &json!({"kind": "assistant_text", "text": "x"}),
        )
        .await
        .unwrap();
    add(root.id.clone(), calling(&["a"])).await;
    add(child.id.clone(), user("two")).await;
    add(child.id.clone(), assistant("ok")).await;
    add(sibling.id.clone(), user("other")).await;
    add(grandchild.id.clone(), user("three")).await;

    assert_eq!(
        history::load(&store, &grandchild.id).await.unwrap(),
        vec![
            user("one"),
            calling(&["a"]),
            tool("a", INTERRUPTED),
            user("two"),
            assistant("ok"),
        ]
    );
    assert!(history::load(&store, &root.id).await.unwrap().is_empty());
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p dots-core --test local_history_test`
Expected: FAIL to compile — unresolved import: `could not find history in local`.

- [ ] **Step 3: Implement**

`crates/dots-core/src/engine/local/history.rs`:
```rust
//! Rebuilds a local-engine conversation from the `message` events of earlier runs.

use serde_json::{json, Value};

use crate::store::Store;
use crate::Result;

/// Tool result recorded for a tool call whose run stopped before it answered.
pub const INTERRUPTED: &str = "This tool call did not complete because the run stopped.";

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
```

In `crates/dots-core/src/engine/local/mod.rs` add `pub mod history;` between `pub mod client;` and `pub mod tools;`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p dots-core --test local_history_test`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/dots-core/src/engine/local crates/dots-core/tests/local_history_test.rs
git commit -m "feat(core): rebuild and repair local conversations from message events"
```

---

### Task 7: The local engine loop

**Files:**
- Modify: `crates/dots-core/src/engine/mod.rs` (`EngineEvent::Message` + its `kind()`)
- Modify: `crates/dots-core/src/engine/local/mod.rs` (full engine)
- Test: `crates/dots-core/tests/local_engine_test.rs`

**Interfaces:**
- Consumes: `tools::{prepare, run, schemas, PreparedCall, ToolOutput}` (Tasks 2–4), `client::{ChatClient, ToolCallRequest, DEFAULT_RETRY_DELAYS}` (Task 5), `history::load` (Task 6), `Engine`, `RunContext`, `PermissionGate`, `PermissionOutcome` (Plan 1).
- Produces:
  - `EngineEvent::Message { message: Value }`, kind `"message"`. Plan 3's run view should hide these (they duplicate `assistant_text` / `tool_call` / `tool_result`) or use them for a raw view.
  - `LocalEngine::new(store: Store, gate: Arc<dyn PermissionGate>)`, `LocalEngine::with_retry_delays(store, gate, retry_delays: Vec<Duration>)`; `impl Engine for LocalEngine`.
- Event order per run: `session_started` (session id = `ctx.session_id`, or the run id for a new lineage) → `message` (system, only when there is no history) → `message` (user = `ctx.prompt`) → per turn: `usage`, `message` (assistant), `assistant_text` (if non-empty), then per tool call `tool_call` (tool = alias, or the raw name if `prepare` failed), `tool_result`, `message` (tool) → `finished { summary = final text }` or `failed { error }`. `max_turns` exhausted → `failed { "error_max_turns: stopped after N turns" }`. On cancel the engine stops without a result for the open call.

- [ ] **Step 1: Write the failing tests**

`crates/dots-core/tests/local_engine_test.rs`:
```rust
mod common;
mod llm;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use common::{spec, temp_store, wait_status};
use dots_core::approvals::ApprovalHub;
use dots_core::engine::local::LocalEngine;
use dots_core::engine::Engine;
use dots_core::events::new_bus;
use dots_core::model::{
    Dot, EngineKind, NewRun, Run, RunEventRecord, RunStatus, TriggerKind, WorkspaceMode,
};
use dots_core::policy::{Policy, Preset};
use dots_core::runner::{Runner, RunnerConfig};
use dots_core::store::Store;
use llm::{call, calls, completion, messages, text, tool_result, Llm};
use serde_json::{json, Value};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;
use wiremock::ResponseTemplate;

struct Env {
    _dir: TempDir,
    ws: PathBuf,
    store: Store,
    runner: Arc<Runner>,
}

async fn env() -> Env {
    let (dir, store) = temp_store().await;
    // The workspace gets its own folder: the store's database lives in `dir`.
    let ws = dir.path().join("ws");
    std::fs::create_dir(&ws).unwrap();
    std::fs::write(ws.join("README.md"), "hello\n").unwrap();
    let bus = new_bus();
    let hub = Arc::new(ApprovalHub::new(store.clone(), bus.clone()));
    let engine = LocalEngine::with_retry_delays(store.clone(), hub, vec![Duration::ZERO; 2]);
    let mut engines: HashMap<EngineKind, Arc<dyn Engine>> = HashMap::new();
    engines.insert(EngineKind::Local, Arc::new(engine));
    let runner = Runner::new(
        store.clone(),
        bus,
        engines,
        RunnerConfig {
            max_concurrent: 2,
            mcp_url: "http://127.0.0.1:1/mcp".into(),
            worktrees_dir: dir.path().join("wt"),
            cancel_grace: Duration::from_millis(500),
        },
    );
    runner.spawn_dispatcher(CancellationToken::new());
    Env {
        _dir: dir,
        ws,
        store,
        runner,
    }
}

async fn local_dot(e: &Env, llm: &Llm, preset: Preset) -> Dot {
    let mut s = spec("loco");
    s.workdir = e.ws.to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(llm.endpoint());
    s.model = "test-model".into();
    s.policy = Policy::preset(preset);
    e.store.create_dot(&s).await.unwrap()
}

async fn run(e: &Env, dot: &Dot) -> Run {
    e.runner
        .enqueue(NewRun::new(&dot.id, TriggerKind::Manual))
        .await
        .unwrap()
}

async fn events(e: &Env, run_id: &str, kind: &str) -> Vec<RunEventRecord> {
    e.store
        .list_events(run_id, 0)
        .await
        .unwrap()
        .into_iter()
        .filter(|ev| ev.kind == kind)
        .collect()
}

#[tokio::test]
async fn reads_a_file_then_finishes() {
    let e = env().await;
    let llm = Llm::start(vec![
        call("c1", "read_file", json!({"file_path": "README.md"})),
        text("The README says hello."),
    ])
    .await;
    let dot = local_dot(&e, &llm, Preset::Sandboxed).await;
    let r = run(&e, &dot).await;
    let done = wait_status(&e.store, &r.id, RunStatus::Succeeded, 10).await;
    assert_eq!(done.summary.as_deref(), Some("The README says hello."));
    assert_eq!(done.session_id.as_deref(), Some(r.id.as_str()));
    assert_eq!((done.tokens_in, done.tokens_out), (20, 10));

    let reqs = llm.requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0]["model"], "test-model");
    assert_eq!(reqs[0]["stream"], false);
    assert_eq!(reqs[0]["tools"].as_array().unwrap().len(), 7);
    let first = messages(&reqs[0]);
    assert_eq!(first[0]["role"], "system");
    assert_eq!(first[1]["role"], "user");
    assert!(first[1]["content"]
        .as_str()
        .unwrap()
        .contains("Do the thing."));
    assert_eq!(tool_result(&reqs[1], "c1"), "hello\n");

    let call_ev = &events(&e, &r.id, "tool_call").await[0];
    assert_eq!(call_ev.data["tool"], "Read");
    assert_eq!(call_ev.data["input"], json!({"file_path": "README.md"}));
    let roles: Vec<Value> = events(&e, &r.id, "message")
        .await
        .iter()
        .map(|ev| ev.data["message"]["role"].clone())
        .collect();
    assert_eq!(
        roles,
        vec!["system", "user", "assistant", "tool", "assistant"]
    );
}

#[tokio::test]
async fn denied_and_malformed_calls_are_reported_to_the_model() {
    let e = env().await;
    let llm = Llm::start(vec![
        calls(&[
            (
                "w",
                "write_file",
                json!({"file_path": "x.txt", "content": "x"}),
            ),
            ("u", "fly", json!({})),
            ("o", "read_file", json!({"file_path": "../secret"})),
        ]),
        completion(json!({"role": "assistant", "content": "", "tool_calls": [
            {"id": "bad", "type": "function",
             "function": {"name": "read_file", "arguments": "{not json"}},
            {"type": "function",
             "function": {"name": "list_dir", "arguments": {"path": "."}}}
        ]})),
        text("gave up"),
    ])
    .await;
    let dot = local_dot(&e, &llm, Preset::ReadOnly).await;
    let r = run(&e, &dot).await;
    wait_status(&e.store, &r.id, RunStatus::Succeeded, 10).await;
    let reqs = llm.requests();
    assert!(tool_result(&reqs[1], "w").contains("Denied by the policy"));
    assert!(tool_result(&reqs[1], "u").contains("unknown tool 'fly'"));
    assert!(tool_result(&reqs[1], "o").contains(".."));
    assert!(!e.ws.join("x.txt").exists());
    assert!(tool_result(&reqs[2], "bad").contains("not valid JSON"));
    assert_eq!(tool_result(&reqs[2], "call_2_1"), "README.md");
    let results = events(&e, &r.id, "tool_result").await;
    assert_eq!(results.len(), 5);
    assert_eq!(results[0].data["is_error"], true);
}

#[tokio::test]
async fn max_turns_fails_the_run() {
    let e = env().await;
    let llm = Llm::start(vec![
        call("a", "list_dir", json!({})),
        call("b", "list_dir", json!({})),
    ])
    .await;
    let mut dot = local_dot(&e, &llm, Preset::Sandboxed).await;
    dot.spec.max_turns = 2;
    e.store.update_dot(&dot.id, &dot.spec).await.unwrap();
    let r = run(&e, &dot).await;
    let failed = wait_status(&e.store, &r.id, RunStatus::Failed, 10).await;
    assert_eq!(
        failed.error.as_deref(),
        Some("error_max_turns: stopped after 2 turns")
    );
    assert_eq!(llm.requests().len(), 2);
}

#[tokio::test]
async fn endpoint_failures_fail_the_run_with_the_reason() {
    let e = env().await;
    let llm = Llm::start(vec![
        ResponseTemplate::new(404).set_body_json(json!({"error": "model not loaded"}))
    ])
    .await;
    let dot = local_dot(&e, &llm, Preset::Sandboxed).await;
    let r = run(&e, &dot).await;
    let failed = wait_status(&e.store, &r.id, RunStatus::Failed, 10).await;
    let err = failed.error.unwrap();
    assert!(
        err.contains("404") && err.contains("model not loaded"),
        "{err}"
    );
}

#[tokio::test]
async fn cancel_during_a_request_cancels_the_run() {
    let e = env().await;
    let slow = text("late").set_delay(Duration::from_secs(30));
    let llm = Llm::start(vec![slow]).await;
    let dot = local_dot(&e, &llm, Preset::Sandboxed).await;
    let r = run(&e, &dot).await;
    llm.wait_requests(1, 10).await;
    e.runner.cancel(&r.id).await.unwrap();
    wait_status(&e.store, &r.id, RunStatus::Cancelled, 5).await;
}

#[tokio::test]
async fn ask_parks_the_call_and_the_run_awaits_approval() {
    let e = env().await;
    let llm = Llm::start(vec![
        call("c1", "shell", json!({"command": "git log -1"})),
        text("Waiting for approval."),
    ])
    .await;
    let mut dot = local_dot(&e, &llm, Preset::Sandboxed).await;
    dot.spec.approval_wait_secs = 0;
    e.store.update_dot(&dot.id, &dot.spec).await.unwrap();
    let r = run(&e, &dot).await;
    wait_status(&e.store, &r.id, RunStatus::AwaitingApproval, 10).await;
    let approvals = e.store.approvals_for_run(&r.id).await.unwrap();
    assert_eq!(approvals.len(), 1);
    assert_eq!(approvals[0].tool, "Bash");
    assert_eq!(approvals[0].input, json!({"command": "git log -1"}));
    let reqs = llm.requests();
    assert!(tool_result(&reqs[1], "c1").contains("Queued for human approval"));
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p dots-core --test local_engine_test`
Expected: FAIL to compile — unresolved import: `no LocalEngine in engine::local`.

- [ ] **Step 3: Add the event**

In `crates/dots-core/src/engine/mod.rs`, add to `EngineEvent` after `Raw { line: String },`:
```rust
    /// One chat message of a local-engine conversation (system, user, assistant or tool), in
    /// OpenAI chat format. Persisted so a resume or follow-up can rebuild the conversation.
    Message {
        message: Value,
    },
```
and to `kind()` after the `Raw` arm:
```rust
            Self::Message { .. } => "message",
```

- [ ] **Step 4: Implement the engine**

Replace `crates/dots-core/src/engine/local/mod.rs` with:
```rust
//! Local engine: a tool loop against an OpenAI-compatible chat endpoint (Ollama, LM Studio).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::sync::mpsc;

use crate::engine::{Engine, EngineEvent, PermissionGate, PermissionOutcome, RunContext};
use crate::store::Store;
use crate::{Error, Result};

use self::client::{ChatClient, ToolCallRequest, DEFAULT_RETRY_DELAYS};
use self::tools::{PreparedCall, ToolOutput};

pub mod client;
pub mod history;
pub mod tools;

const SYSTEM_PROMPT: &str = "You are a dot: an autonomous agent working unattended in a \
    workspace on the user's computer. Use the tools to inspect and change files and to run \
    commands; paths are relative to the workspace root. When the task is done, reply with a \
    short summary and no tool calls.";

pub struct LocalEngine {
    store: Store,
    gate: Arc<dyn PermissionGate>,
    client: ChatClient,
}

impl LocalEngine {
    pub fn new(store: Store, gate: Arc<dyn PermissionGate>) -> Self {
        Self::with_retry_delays(store, gate, DEFAULT_RETRY_DELAYS.to_vec())
    }

    pub fn with_retry_delays(
        store: Store,
        gate: Arc<dyn PermissionGate>,
        retry_delays: Vec<Duration>,
    ) -> Self {
        Self {
            store,
            gate,
            client: ChatClient::new(retry_delays),
        }
    }
}

#[async_trait]
impl Engine for LocalEngine {
    async fn start(&self, ctx: RunContext) -> Result<mpsc::Receiver<EngineEvent>> {
        let endpoint = ctx
            .dot
            .spec
            .endpoint_url
            .clone()
            .filter(|u| !u.trim().is_empty())
            .ok_or_else(|| Error::Invalid("the local engine requires endpoint_url".into()))?;
        let messages = match ctx.session_id {
            Some(_) => history::load(&self.store, &ctx.run_id).await?,
            None => Vec::new(),
        };
        let (tx, rx) = mpsc::channel(256);
        let session = Session {
            gate: self.gate.clone(),
            client: self.client.clone(),
            endpoint,
            ctx,
            tx,
            messages,
        };
        tokio::spawn(session.run());
        Ok(rx)
    }
}

/// One run's conversation. Every method returns `false` once the run should stop (cancelled,
/// or the runner stopped listening).
struct Session {
    gate: Arc<dyn PermissionGate>,
    client: ChatClient,
    endpoint: String,
    ctx: RunContext,
    tx: mpsc::Sender<EngineEvent>,
    messages: Vec<Value>,
}

impl Session {
    async fn emit(&self, ev: EngineEvent) -> bool {
        self.tx.send(ev).await.is_ok()
    }

    /// Appends a message to the conversation and records it as a `message` event.
    async fn push(&mut self, message: Value) -> bool {
        self.messages.push(message.clone());
        self.emit(EngineEvent::Message { message }).await
    }

    async fn run(mut self) {
        // A local session is named after the root run of its lineage.
        let session_id = self
            .ctx
            .session_id
            .clone()
            .unwrap_or_else(|| self.ctx.run_id.clone());
        if !self.emit(EngineEvent::SessionStarted { session_id }).await {
            return;
        }
        if self.messages.is_empty()
            && !self
                .push(json!({"role": "system", "content": SYSTEM_PROMPT}))
                .await
        {
            return;
        }
        let prompt = self.ctx.prompt.clone();
        if !self.push(json!({"role": "user", "content": prompt})).await {
            return;
        }
        let tools = tools::schemas();
        let max_turns = self.ctx.dot.spec.max_turns;
        for turn in 1..=max_turns {
            let body = json!({
                "model": self.ctx.dot.spec.model,
                "messages": self.messages,
                "tools": tools,
                "stream": false
            });
            let completion = match self
                .client
                .complete(&self.endpoint, &body, turn, &self.ctx.cancel)
                .await
            {
                Ok(Some(c)) => c,
                Ok(None) => return,
                Err(e) => {
                    self.emit(EngineEvent::Failed {
                        error: e.to_string(),
                    })
                    .await;
                    return;
                }
            };
            let usage = EngineEvent::Usage {
                tokens_in: completion.tokens_in,
                tokens_out: completion.tokens_out,
            };
            if !self.emit(usage).await || !self.push(completion.message.clone()).await {
                return;
            }
            let text = completion.content.trim().to_string();
            if !text.is_empty()
                && !self
                    .emit(EngineEvent::AssistantText { text: text.clone() })
                    .await
            {
                return;
            }
            if completion.tool_calls.is_empty() {
                self.emit(EngineEvent::Finished { summary: text }).await;
                return;
            }
            for call in &completion.tool_calls {
                if !self.handle(call).await {
                    return;
                }
            }
        }
        self.emit(EngineEvent::Failed {
            error: format!("error_max_turns: stopped after {max_turns} turns"),
        })
        .await;
    }

    /// Checks, gates and runs one tool call, then records its result.
    async fn handle(&mut self, call: &ToolCallRequest) -> bool {
        if self.ctx.cancel.is_cancelled() {
            return false;
        }
        let prepared = tools::prepare(&call.name, &call.arguments, &self.ctx.workspace);
        let (tool, input) = match &prepared {
            Ok(p) => (p.alias.to_string(), p.input.clone()),
            Err(_) => (call.name.clone(), call.arguments.clone()),
        };
        let shown = EngineEvent::ToolCall {
            id: call.id.clone(),
            tool,
            input,
        };
        if !self.emit(shown).await {
            return false;
        }
        let output = match prepared {
            Err(e) => ToolOutput::err(e),
            Ok(p) => {
                let checked = tokio::select! {
                    r = self.gate.check(&self.ctx.run_id, p.alias, p.input.clone()) => r,
                    _ = self.ctx.cancel.cancelled() => return false,
                };
                match checked {
                    Ok(PermissionOutcome::Allow { input }) => {
                        let allowed = PreparedCall { input, ..p };
                        tools::run(&allowed, &self.ctx.workspace, &self.ctx.cancel).await
                    }
                    Ok(PermissionOutcome::Deny { message }) => ToolOutput::err(message),
                    Err(e) => ToolOutput::err(format!("permission check failed: {e}")),
                }
            }
        };
        // A cancel that stopped the tool leaves the call unanswered; `history::repair` fills it.
        if self.ctx.cancel.is_cancelled() {
            return false;
        }
        let result = EngineEvent::ToolResult {
            id: call.id.clone(),
            output: output.output.clone(),
            is_error: output.is_error,
        };
        self.emit(result).await
            && self
                .push(json!({
                    "role": "tool",
                    "tool_call_id": call.id,
                    "content": output.output
                }))
                .await
    }
}
```

Notes for the implementer:
- The runner records events in channel order, so every `message` event of a run is stored before the run finishes; `history::load` in a later run therefore sees the whole conversation.
- A denied or parked call's text (`Denied by the policy…`, `Queued for human approval #id…`) goes back to the model as an error result; the model then finishes, and the runner marks the run `awaiting_approval` because a parked approval is open (Plan 1 logic, unchanged).
- The gate gets the alias (`Bash`, `Read`, …) and the prepared input, so the dot's policy, grants and approvals behave exactly as for Claude dots.
- The test workspace is a `ws` subfolder of the temp dir because the test store's `dots.db` lives in the temp dir itself; otherwise `list_dir` would list the database files.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p dots-core --test local_engine_test`
Expected: PASS (6 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/dots-core/src/engine crates/dots-core/tests/local_engine_test.rs
git commit -m "feat(core): local engine tool loop with policy gate and message history"
```

---

### Task 8: Runtime wiring and the endpoint probe

**Files:**
- Modify: `crates/dots-core/src/engine/local/mod.rs` (add `probe`)
- Modify: `crates/dots-core/src/runtime.rs` (imports, `Config.local_retry_delays`, engine registration, `test_endpoint`)
- Test: `crates/dots-core/tests/runtime_local_test.rs`

**Interfaces:**
- Consumes: everything above; `ApprovalHub` (as `Arc<dyn PermissionGate>`), `check_endpoint_url` (Task 1).
- Produces (the API Plan 3 calls):
  - `Config.local_retry_delays: Vec<Duration>` (default `DEFAULT_RETRY_DELAYS`).
  - `Runtime::start` registers `EngineKind::Local` unconditionally.
  - `local::probe(endpoint: &str, model: &str) -> Result<String>` and `Runtime::test_endpoint(&self, endpoint: &str, model: &str) -> Result<String>`: `Ok("<model> returned a well-formed tool call")`; `Err(Invalid)` for a bad URL, an empty model, a text answer, a wrong tool or malformed arguments; `Err(Other)` with the endpoint's message for HTTP errors or no answer within 120 s.

- [ ] **Step 1: Write the failing tests**

`crates/dots-core/tests/runtime_local_test.rs`:
```rust
mod common;
mod llm;

use std::path::PathBuf;
use std::time::Duration;

use common::{spec, wait_status};
use dots_core::events::RuntimeEvent;
use dots_core::model::{DotSpec, EngineKind, RunStatus, TriggerKind, WorkspaceMode};
use dots_core::{Config, Error, Runtime};
use llm::{call, messages, text, tool_result, Llm};
use serde_json::json;
use tempfile::TempDir;
use wiremock::ResponseTemplate;

struct Rt {
    dir: TempDir,
    rt: Runtime,
}

async fn start() -> Rt {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = Config::new(dir.path().join("data"));
    cfg.port = 0;
    cfg.claude_path = Some(PathBuf::from(env!("CARGO_BIN_EXE_fake-claude")));
    cfg.local_retry_delays = vec![Duration::ZERO; 2];
    let rt = Runtime::start(cfg).await.unwrap();
    Rt { dir, rt }
}

fn local_spec(t: &Rt, llm: &Llm, name: &str) -> DotSpec {
    let mut s = spec(name);
    s.workdir = t.dir.path().to_string_lossy().to_string();
    s.workspace_mode = WorkspaceMode::Folder;
    s.engine = EngineKind::Local;
    s.endpoint_url = Some(format!("{}/v1", llm.endpoint()));
    s.model = "test-model".into();
    s
}

#[cfg(windows)]
const ECHO: &str = "Write-Output approved-ok";
#[cfg(not(windows))]
const ECHO: &str = "echo approved-ok";

#[tokio::test]
async fn live_approval_lets_the_command_run() {
    let t = start().await;
    let llm = Llm::start(vec![
        call("c1", "shell", json!({"command": ECHO})),
        text("ran it"),
    ])
    .await;
    let mut s = local_spec(&t, &llm, "live");
    s.approval_wait_secs = 30;
    let dot = t.rt.create_dot(s).await.unwrap();
    let mut events = t.rt.subscribe();
    let r = t.rt.run_now(&dot.id).await.unwrap();
    let approval = loop {
        if let RuntimeEvent::ApprovalRequested { approval } = events.recv().await.unwrap() {
            break approval;
        }
    };
    assert_eq!(approval.tool, "Bash");
    t.rt.decide_approval(&approval.id, true, None)
        .await
        .unwrap();
    wait_status(t.rt.store(), &r.id, RunStatus::Succeeded, 30).await;
    assert_eq!(tool_result(&llm.requests()[1], "c1").trim(), "approved-ok");
    t.rt.shutdown();
}

#[tokio::test]
async fn parked_approval_resumes_and_the_grant_allows_one_retry() {
    let t = start().await;
    let llm = Llm::start(vec![
        call("c1", "shell", json!({"command": ECHO})),
        text("Queued; nothing else to do."),
        call("c2", "shell", json!({"command": ECHO})),
        text("Done after approval."),
    ])
    .await;
    let mut s = local_spec(&t, &llm, "parked");
    s.approval_wait_secs = 0;
    let dot = t.rt.create_dot(s).await.unwrap();
    let parent = t.rt.run_now(&dot.id).await.unwrap();
    wait_status(t.rt.store(), &parent.id, RunStatus::AwaitingApproval, 10).await;
    let approval =
        t.rt.store()
            .list_pending_approvals()
            .await
            .unwrap()
            .remove(0);
    t.rt.decide_approval(&approval.id, true, None)
        .await
        .unwrap();

    let mut child = None;
    for _ in 0..200 {
        let runs = t.rt.store().list_runs(Some(&dot.id), 10).await.unwrap();
        if let Some(c) = runs.into_iter().find(|r| r.trigger == TriggerKind::Resume) {
            child = Some(c);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let child = child.expect("resume run");
    let child = wait_status(t.rt.store(), &child.id, RunStatus::Succeeded, 30).await;
    assert_eq!(child.session_id.as_deref(), Some(parent.id.as_str()));
    assert_eq!(child.summary.as_deref(), Some("Done after approval."));

    let reqs = llm.requests();
    assert_eq!(reqs.len(), 4);
    let resumed = messages(&reqs[2]);
    let roles: Vec<&str> = resumed
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(
        roles,
        vec!["system", "user", "assistant", "tool", "assistant", "user"]
    );
    assert!(resumed[3]["content"]
        .as_str()
        .unwrap()
        .contains("Queued for human approval"));
    assert!(resumed[5]["content"].as_str().unwrap().contains("granted"));
    assert_eq!(tool_result(&reqs[3], "c2").trim(), "approved-ok");
    // The retry used the grant: no second approval was asked.
    assert!(t
        .rt
        .store()
        .approvals_for_run(&child.id)
        .await
        .unwrap()
        .is_empty());
    t.rt.shutdown();
}

#[tokio::test]
async fn chat_follow_up_continues_the_conversation() {
    let t = start().await;
    let llm = Llm::start(vec![text("hello there"), text("still here")]).await;
    let dot =
        t.rt.create_dot(local_spec(&t, &llm, "chatty"))
            .await
            .unwrap();
    let first = t.rt.chat(&dot.id, "hi", None).await.unwrap();
    let first = wait_status(t.rt.store(), &first.id, RunStatus::Succeeded, 10).await;
    let follow = t.rt.chat(&dot.id, "again?", Some(&first.id)).await.unwrap();
    wait_status(t.rt.store(), &follow.id, RunStatus::Succeeded, 10).await;

    let sent = messages(&llm.requests()[1]);
    let contents: Vec<&str> = sent
        .iter()
        .map(|m| m["content"].as_str().unwrap())
        .collect();
    assert_eq!(sent.len(), 4);
    assert!(contents[1].contains("hi"));
    assert_eq!(contents[2], "hello there");
    assert_eq!(contents[3], "again?");
    t.rt.shutdown();
}

#[tokio::test]
async fn test_endpoint_checks_for_a_well_formed_tool_call() {
    let t = start().await;
    let good = Llm::start(vec![call("p", "echo", json!({"text": "ping"}))]).await;
    let msg =
        t.rt.test_endpoint(&good.endpoint(), "test-model")
            .await
            .unwrap();
    assert!(msg.contains("well-formed"), "{msg}");
    assert_eq!(good.requests()[0]["tools"][0]["function"]["name"], "echo");

    let chatty = Llm::start(vec![text("ping!")]).await;
    let err =
        t.rt.test_endpoint(&chatty.endpoint(), "m")
            .await
            .unwrap_err();
    assert!(err.to_string().contains("instead of a tool call"), "{err}");

    let broken = Llm::start(vec![call("p", "echo", json!({"nope": 1}))]).await;
    let err =
        t.rt.test_endpoint(&broken.endpoint(), "m")
            .await
            .unwrap_err();
    assert!(err.to_string().contains("malformed"), "{err}");

    let down = Llm::start(vec![
        ResponseTemplate::new(400).set_body_string("no such model")
    ])
    .await;
    let err = t.rt.test_endpoint(&down.endpoint(), "m").await.unwrap_err();
    assert!(err.to_string().contains("no such model"), "{err}");

    let err =
        t.rt.test_endpoint("https://example.com", "m")
            .await
            .unwrap_err();
    assert!(matches!(err, Error::Invalid(_)), "{err:?}");
    t.rt.shutdown();
}
```

- [ ] **Step 2: Run them to verify they fail**

Run: `cargo test -p dots-core --test runtime_local_test`
Expected: FAIL to compile — `no field local_retry_delays on type Config` and `no method named test_endpoint`.

- [ ] **Step 3: Add the probe**

In `crates/dots-core/src/engine/local/mod.rs` add `use tokio_util::sync::CancellationToken;` after `use tokio::sync::mpsc;`, and insert above `pub struct LocalEngine`:
```rust
/// How long `probe` waits; LM Studio may load the model on the first request.
const PROBE_TIMEOUT: Duration = Duration::from_secs(120);

/// Asks `model` at `endpoint` to call a one-argument `echo` tool and checks that the reply is a
/// well-formed tool call. Returns a short success message; every failure is an error that
/// says what the endpoint or model did instead.
pub async fn probe(endpoint: &str, model: &str) -> Result<String> {
    crate::model::check_endpoint_url(endpoint)?;
    if model.trim().is_empty() {
        return Err(Error::Invalid("model must not be empty".into()));
    }
    let body = json!({
        "model": model,
        "messages": [{
            "role": "user",
            "content": "Call the echo tool with text set to \"ping\". Do not answer with text."
        }],
        "tools": [{
            "type": "function",
            "function": {
                "name": "echo",
                "description": "Echo the given text back.",
                "parameters": {
                    "type": "object",
                    "properties": {"text": {"type": "string"}},
                    "required": ["text"]
                }
            }
        }],
        "stream": false
    });
    let client = ChatClient::new(Vec::new());
    let never = CancellationToken::new();
    let completion =
        tokio::time::timeout(PROBE_TIMEOUT, client.complete(endpoint, &body, 1, &never))
            .await
            .map_err(|_| {
                Error::Other(format!(
                    "{model} did not answer within {} s",
                    PROBE_TIMEOUT.as_secs()
                ))
            })??
            .ok_or_else(|| Error::Other("probe cancelled".into()))?;
    let Some(call) = completion.tool_calls.first() else {
        return Err(Error::Invalid(format!(
            "{model} answered with text instead of a tool call: {}",
            completion.content.trim()
        )));
    };
    if call.name != "echo" {
        return Err(Error::Invalid(format!(
            "{model} called an unknown tool '{}'",
            call.name
        )));
    }
    let args = match &call.arguments {
        Value::String(s) => serde_json::from_str::<Value>(s).unwrap_or(Value::Null),
        other => other.clone(),
    };
    match args.get("text").and_then(Value::as_str) {
        Some(_) => Ok(format!("{model} returned a well-formed tool call")),
        None => Err(Error::Invalid(format!(
            "{model} sent malformed tool arguments: {}",
            call.arguments
        ))),
    }
}
```

- [ ] **Step 4: Wire the runtime**

In `crates/dots-core/src/runtime.rs`:

Imports, after `use crate::engine::claude::ClaudeEngine;`:
```rust
use crate::engine::local::client::DEFAULT_RETRY_DELAYS;
use crate::engine::local::{self, LocalEngine};
```

`Config` gets a field after `claude_env`:
```rust
    /// Delays between attempts when a local endpoint is unreachable or answers 5xx.
    pub local_retry_delays: Vec<Duration>,
```
and `Config::new` sets it after `claude_env: Vec::new(),`:
```rust
            local_retry_delays: DEFAULT_RETRY_DELAYS.to_vec(),
```

In `Runtime::start`, after the `match &claude_program { ... }` block and before `let runner = Runner::new(`:
```rust
        engines.insert(
            EngineKind::Local,
            Arc::new(LocalEngine::with_retry_delays(
                store.clone(),
                hub.clone(),
                cfg.local_retry_delays.clone(),
            )),
        );
```

Above `pub fn shutdown(&self)`:
```rust
    /// Checks that `model` at the OpenAI-compatible `endpoint` answers with a well-formed tool
    /// call (the Settings "Test endpoint" button). Returns a short success message.
    pub async fn test_endpoint(&self, endpoint: &str, model: &str) -> Result<String> {
        local::probe(endpoint, model).await
    }
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p dots-core --test runtime_local_test`
Expected: PASS (4 tests).

- [ ] **Step 6: Run the whole suite, lints and formatting**

Run: `cargo test -p dots-core`
Expected: PASS, 0 failures (179 tests when this plan was checked). `runner_test`'s "engine 'local' is not available" test still passes because it builds its own `Runner` without the local engine.

Run: `cargo clippy -p dots-core --all-targets` and `cargo fmt --check`
Expected: no warnings, no diff.

- [ ] **Step 7: Commit**

```bash
git add crates/dots-core/src crates/dots-core/tests/runtime_local_test.rs
git commit -m "feat(core): register the local engine and add the endpoint probe"
```

---

### Task 9: Manual smoke against LM Studio

**Files:**
- Modify: `crates/dots-core/examples/smoke.rs` (`--engine`, `--endpoint`)
- Create: `docs/verification/2026-10-09-plan2-smoke.md`

**Interfaces:**
- Consumes: `Runtime::test_endpoint`, `EngineKind::parse` (Plan 1 `str_enum!`).

- [ ] **Step 1: Add the flags to the smoke example**

In `crates/dots-core/examples/smoke.rs` replace the doc header with:
```rust
//! Manual end-to-end check against the real `claude` CLI (uses your subscription) or, with
//! `--engine local --endpoint <url>`, a local OpenAI-compatible server (LM Studio, Ollama).
//!
//! cargo run -p dots-core --example smoke -- <workdir> "<instructions>"
//!     [--model haiku] [--wait <approval_wait_secs>] [--preset read-only|sandboxed|trusted] [--folder]
//!     [--engine claude|local] [--endpoint http://127.0.0.1:1234]
```
change the model import to `use dots_core::model::{DotSpec, EngineKind, RunStatus, WorkspaceMode};`, the usage error to:
```rust
        bail!(
            "usage: smoke <workdir> <instructions> [--model m] [--wait secs] [--preset p] \
             [--folder] [--engine claude|local] [--endpoint url]"
        );
```
and after `spec.model = flag(&args, "--model")...;` add:
```rust
    if let Some(engine) = flag(&args, "--engine") {
        spec.engine = EngineKind::parse(&engine)?;
    }
    spec.endpoint_url = flag(&args, "--endpoint");
    if let Some(endpoint) = &spec.endpoint_url {
        println!("probe: {}", rt.test_endpoint(endpoint, &spec.model).await?);
    }
```

Run: `cargo build -p dots-core --example smoke`
Expected: builds with no warnings.

- [ ] **Step 2: Start LM Studio's server with the smoke model**

```bash
~/.lmstudio/bin/lms server start
~/.lmstudio/bin/lms load google/gemma-4-12b-qat
~/.lmstudio/bin/lms ps
```
Expected: the server listens on `http://127.0.0.1:1234`; `lms ps` lists `google/gemma-4-12b-qat`. Answer any prompt `lms load` shows with the defaults.

- [ ] **Step 3: Create a throwaway repo to work in**

```bash
W="$TEMP/dots-smoke-repo"; rm -rf "$W"; mkdir -p "$W" && cd "$W" && git init -q -b main \
  && printf '# smoke repo\n\nTiny repo for the local engine smoke.\n' > README.md \
  && git add -A && git -c user.name=smoke -c user.email=smoke@local commit -qm init && cd -
```

- [ ] **Step 4: Plain run (read-only)**

```bash
env -u TMP cargo run -q -p dots-core --example smoke -- "$TEMP/dots-smoke-repo" \
  "List the files in this repo and summarize README.md in one sentence. Do not modify anything." \
  --engine local --endpoint http://127.0.0.1:1234 --model google/gemma-4-12b-qat --preset read-only < /dev/null
```
Expected: `probe: google/gemma-4-12b-qat returned a well-formed tool call`, then `session_started`, `message`, `tool_call` (`LS`, `Glob` or `Read`), `tool_result`, `assistant_text`, `finished`, and `run … -> succeeded`.
If the probe fails with "answered with text instead of a tool call", the model or its LM Studio prompt template does not emit tool calls: record that, retry with `--model google/gemma-4-e2b`, and record which model worked. (`env -u TMP` keeps Rust's `temp_dir()` on `%TEMP%`; see the Plan 1 smoke notes.)

- [ ] **Step 5: Live approval**

```bash
env -u TMP cargo run -q -p dots-core --example smoke -- "$TEMP/dots-smoke-repo" \
  "Run the shell command: git log -1 --oneline. Then report its output." \
  --engine local --endpoint http://127.0.0.1:1234 --model google/gemma-4-12b-qat --wait 120
```
Expected: `APPROVAL … Bash {"command":"git log -1 --oneline"}`; type `y` and Enter within 120 s; the `tool_result` shows the commit line; the run succeeds.

- [ ] **Step 6: Park and resume**

Same command with `--wait 5`; wait about 15 s before typing `y`.
Expected: the first run ends `awaiting_approval` (its tool result says "Queued for human approval"); after `y` a `resume` run starts, the model re-issues the call, the grant allows it once without a second APPROVAL prompt, and the resume run succeeds. If the model changes the command on retry, a second APPROVAL appears; that is accepted behavior (spec §5) — record it.

- [ ] **Step 7: Record the results and free the model**

Create `docs/verification/2026-10-09-plan2-smoke.md` with the outcome of each step, in the style of `docs/verification/2026-10-08-plan1-smoke.md`:
```markdown
# Plan 2 smoke verification

- Date: <date of the run>
- LM Studio: <`lms version` output>; model: <model that passed the probe>
- Step 4 plain run: <pass/fail> — notes: <events seen, tool used, summary>
- Step 5 live approval: <pass/fail> — notes: <command approved, tool result>
- Step 6 park + resume: <pass/fail> — notes: <parent status, resume run status, whether a second approval appeared>
- Issues found: <none, or what broke and the fix commit>
```
Then free the memory:
```bash
~/.lmstudio/bin/lms unload --all
~/.lmstudio/bin/lms server stop
```

- [ ] **Step 8: Commit**

```bash
git add crates/dots-core/examples/smoke.rs docs/verification/2026-10-09-plan2-smoke.md
git commit -m "docs(verification): record the local engine smoke against LM Studio"
```
