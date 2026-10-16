use dots_core::library::McpServerConfig;
use dots_core::mcp::{fingerprint, McpError, McpTool};
use dots_core::util::json_hash;
use dots_core::Error;
use serde_json::{json, Value};

fn tool(v: Value) -> McpTool {
    serde_json::from_value(v).unwrap()
}

#[test]
fn variables_are_found_and_expanded_from_the_environment() {
    let cfg: McpServerConfig = serde_json::from_value(json!({
        "transport": "stdio", "command": "${TOOLS}/srv", "args": ["--org", "${ORG}", "${ORG}"],
        "cwd": "${HOME_DIR}", "env": { "SF_TOKEN": "${SF_TOKEN}", "MODE": "readonly", "X": "${" }
    }))
    .unwrap();
    assert_eq!(
        cfg.variables(),
        ["HOME_DIR", "ORG", "SF_TOKEN"].map(String::from)
    );
    let env = |k: &str| match k {
        "TOOLS" => Some("C:/t".to_string()),
        "ORG" => Some("acme".to_string()),
        "HOME_DIR" => Some("C:/h".to_string()),
        "SF_TOKEN" => Some("s3cret".to_string()),
        _ => None,
    };
    let McpServerConfig::Stdio {
        command,
        args,
        cwd,
        env: vars,
    } = cfg.expanded(&env).unwrap()
    else {
        panic!()
    };
    // The spec reads variables in env values, args, cwd, url and headers: not in the command.
    assert_eq!(command, "${TOOLS}/srv");
    assert_eq!(args, ["--org", "acme", "acme"]);
    assert_eq!(cwd.as_deref(), Some("C:/h"));
    assert_eq!(vars["SF_TOKEN"], "s3cret");
    assert_eq!(vars["MODE"], "readonly");
    assert_eq!(vars["X"], "${", "an unclosed reference stays as written");

    let http: McpServerConfig = serde_json::from_value(json!({
        "transport": "http", "url": "https://${HOST}/mcp",
        "headers": { "Authorization": "Bearer ${EXAMPLE_TOKEN}" }
    }))
    .unwrap();
    assert_eq!(
        http.variables(),
        ["EXAMPLE_TOKEN", "HOST"].map(String::from)
    );
    let err = http
        .expanded(&|k| (k == "HOST").then(|| "h".into()))
        .unwrap_err();
    assert!(
        matches!(&err, Error::Invalid(m) if m == "EXAMPLE_TOKEN is not set"),
        "{err:?}"
    );
}

#[test]
fn a_tool_keeps_what_dots_shows_and_drops_the_rest() {
    let t = tool(json!({
        "name": "search", "title": "Search", "description": "Finds things.",
        "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}}},
        "outputSchema": {"type": "object"}, "annotations": {"readOnlyHint": true},
        "_meta": {"x": 1}
    }));
    assert_eq!(t.name, "search");
    assert_eq!(t.title.as_deref(), Some("Search"));
    assert_eq!(t.description.as_deref(), Some("Finds things."));
    assert!(t.read_only());
    assert_eq!(
        serde_json::to_value(&t).unwrap(),
        json!({
            "name": "search", "title": "Search", "description": "Finds things.",
            "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}}},
            "annotations": {"readOnlyHint": true}
        })
    );

    // A tool without a schema takes no arguments; only `readOnlyHint: true` is read-only.
    let bare = tool(json!({"name": "ping"}));
    assert_eq!(bare.input_schema, json!({"type": "object"}));
    assert!(!bare.read_only());
    for hint in [json!(false), json!("true"), json!(1), Value::Null] {
        let t = tool(json!({"name": "w", "annotations": {"readOnlyHint": hint}}));
        assert!(!t.read_only(), "{hint}");
    }
}

#[test]
fn the_fingerprint_hashes_the_sorted_tools_in_canonical_json() {
    let b = json!({"name": "b", "description": "B.", "inputSchema": {"type": "object"},
                   "annotations": {"readOnlyHint": true}, "title": "Bee",
                   "outputSchema": {"type": "string"}, "_meta": {"v": 1}});
    let a = json!({"name": "a", "inputSchema": {"required": [], "type": "object"}});
    let tools = vec![tool(b.clone()), tool(a.clone())];
    // Sorted by name; `title`, `outputSchema` and `_meta` are not part of it; a field the
    // server did not send is left out.
    assert_eq!(
        fingerprint(&tools),
        json_hash(&json!([
            {"name": "a", "inputSchema": {"type": "object", "required": []}},
            {"name": "b", "description": "B.", "inputSchema": {"type": "object"},
             "annotations": {"readOnlyHint": true}}
        ]))
    );
    assert_eq!(fingerprint(&tools).len(), 64);
    assert_eq!(
        fingerprint(&tools),
        fingerprint(&[tool(a.clone()), tool(b.clone())]),
        "the order of the list does not matter"
    );

    let base = fingerprint(&tools);
    let with = |key: &str, value: Value| {
        let mut changed = b.clone();
        changed[key] = value;
        fingerprint(&[tool(a.clone()), tool(changed)])
    };
    assert_eq!(with("title", json!("Other")), base);
    assert_eq!(with("outputSchema", json!({})), base);
    assert_eq!(with("_meta", json!({"v": 2})), base);
    assert_ne!(with("description", json!("Changed.")), base);
    assert_ne!(with("inputSchema", json!({"type": "object", "x": 1})), base);
    assert_ne!(with("annotations", json!({"readOnlyHint": false})), base);
    assert_ne!(fingerprint(&[tool(a.clone())]), base, "a removed tool");
    assert_ne!(fingerprint(&[]), base);
}

#[test]
fn errors_say_what_happened_without_the_server_name() {
    let cases = [
        (
            McpError::Start("SF_TOKEN is not set".into()),
            "could not start: SF_TOKEN is not set",
        ),
        (McpError::Timeout, "did not answer"),
        (McpError::Stopped, "the server stopped"),
        (McpError::Cancelled, "cancelled"),
        (
            McpError::Server {
                code: -32602,
                message: "unknown tool: x".into(),
            },
            "unknown tool: x (error -32602)",
        ),
        (
            McpError::Protocol("the server answered HTTP 500".into()),
            "the server answered HTTP 500",
        ),
    ];
    for (err, text) in cases {
        assert_eq!(err.to_string(), text);
        assert!(matches!(Error::from(err), Error::Other(m) if m == text));
    }
}

#[cfg(windows)]
#[test]
fn a_bare_command_is_found_on_the_path_with_its_extension() {
    use dots_core::mcp::resolve_program;
    use std::ffi::OsString;
    use std::path::PathBuf;

    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("npx.cmd"), "@echo off\r\n").unwrap();
    std::fs::write(other.path().join("npx.exe"), "").unwrap();
    let path = std::env::join_paths([dir.path(), other.path()]).unwrap();
    let pathext = OsString::from(".COM;.EXE;.BAT;.CMD");

    // The first folder with any match wins, as cmd.exe finds a program.
    assert_eq!(
        resolve_program("npx", Some(&path), Some(&pathext)),
        dir.path().join("npx.cmd")
    );
    // A command with a folder or an extension, or one not found, is left as written.
    for command in ["node.exe", "C:/tools/x", "missing"] {
        assert_eq!(
            resolve_program(command, Some(&path), Some(&pathext)),
            PathBuf::from(command)
        );
    }
    // Without PATHEXT the usual list is used.
    assert_eq!(
        resolve_program("npx", Some(&path), None),
        dir.path().join("npx.cmd")
    );
}
