//! Built-in tools of the local engine. Argument names match the Claude tools they alias, so
//! policy path specs, command rules, grants and approvals treat local and Claude calls alike.

use std::path::{Component, Path, PathBuf, Prefix};

use serde_json::{json, Map, Value};
use tokio_util::sync::CancellationToken;

mod fs;
mod search;
mod shell;

/// Tool output sent back to the model is capped at this many bytes.
pub const MAX_OUTPUT: usize = 30 * 1024;
/// `read_file` and `edit_file` refuse larger files rather than load them into memory.
pub const MAX_READ_BYTES: u64 = 10 * 1024 * 1024;
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

#[derive(Clone, Copy, PartialEq)]
enum Arg {
    Str,
    /// A string that `prepare` replaces with its canonical workspace-relative form.
    PathStr,
    /// A non-negative integer.
    Count,
    Bool,
}

use Arg::{Bool, Count, PathStr, Str};

/// (argument, type, required)
type Params = &'static [(&'static str, Arg, bool)];

/// The arguments of each local tool. Every other argument is dropped: the policy reads the
/// first of `file_path`/`path`, so an extra one could stand in for the real target.
const ARGS: &[(&str, Params)] = &[
    (
        "read_file",
        &[
            ("file_path", PathStr, true),
            ("offset", Count, false),
            ("limit", Count, false),
        ],
    ),
    (
        "write_file",
        &[("file_path", PathStr, true), ("content", Str, true)],
    ),
    (
        "edit_file",
        &[
            ("file_path", PathStr, true),
            ("old_string", Str, true),
            ("new_string", Str, true),
        ],
    ),
    ("list_dir", &[("path", PathStr, false)]),
    ("glob", &[("pattern", Str, true), ("path", PathStr, false)]),
    (
        "grep",
        &[
            ("pattern", Str, true),
            ("path", PathStr, false),
            ("glob", Str, false),
            ("ignore_case", Bool, false),
        ],
    ),
    (
        "shell",
        &[("command", Str, true), ("timeout", Count, false)],
    ),
];

/// Checks a model tool call before the policy sees it: a known tool, arguments that form a
/// JSON object (sent as a JSON string, as an object, or empty), only the tool's own arguments
/// with the right types, and every path inside the workspace. Paths are replaced by their
/// canonical workspace-relative form, so the policy, approvals and grants judge exactly what
/// the tool will touch (padding trimmed, links resolved). An empty or null optional argument is
/// dropped. The error text is returned to the model as the tool result.
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
    let args = ARGS
        .iter()
        .find(|(n, _)| *n == name)
        .map_or(&[][..], |(_, a)| *a);
    input.retain(|key, _| args.iter().any(|(k, _, _)| k == key));
    let mut root = None;
    for &(key, kind, required) in args {
        let blank = match input.get(key) {
            None | Some(Value::Null) => true,
            Some(Value::String(s)) => !required && kind == PathStr && s.trim().is_empty(),
            Some(_) => false,
        };
        // Every required argument is a string.
        if required && (blank || !input[key].is_string()) {
            return Err(format!("{name} requires the string argument '{key}'"));
        }
        if blank {
            input.remove(key);
            continue;
        }
        let value = &input[key];
        let ok = match kind {
            Str | PathStr => value.is_string(),
            Count => value.is_u64(),
            Bool => value.is_boolean(),
        };
        if !ok {
            let expected = match kind {
                Str | PathStr => "a string",
                Count => "a non-negative integer",
                Bool => "a boolean",
            };
            return Err(format!("{name}: '{key}' must be {expected}"));
        }
        if kind == PathStr {
            let root = match &root {
                Some(r) => r,
                None => root.insert(
                    std::fs::canonicalize(workspace)
                        .map_err(|e| format!("workspace is not accessible: {e}"))?,
                ),
            };
            let path = confine(root, value.as_str().unwrap_or_default())?;
            input.insert(key.to_string(), Value::String(display(root, &path)));
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
        "glob" => search::glob(&root, input).await,
        "grep" => search::grep(&root, input).await,
        "shell" => shell::shell(&root, input, cancel).await,
        other => ToolOutput::err(format!("unknown tool '{other}'")),
    }
}

/// Resolves `raw` (relative to the workspace, or absolute) to a path inside the workspace.
/// Symlinks and junctions are resolved through the longest existing ancestor, so a link that
/// points outside is rejected; `..`, NTFS stream names (`a.txt:s`), network/device paths (`\\host\share`), drive-relative paths (`C:foo`), dangling links and empty paths are refused.
pub fn confine(workspace: &Path, raw: &str) -> Result<PathBuf, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("path must not be empty".into());
    }
    let given = Path::new(trimmed);
    for c in given.components() {
        match c {
            Component::ParentDir => return Err(format!("'..' is not allowed in paths: {raw}")),
            // Only local drive prefixes: UNC, device and other verbatim paths would make the
            // filesystem contact another host or device before the permission gate runs.
            Component::Prefix(p)
                if !matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_)) =>
            {
                return Err(format!("network and device paths are not allowed: {raw}"))
            }
            Component::Normal(n) if n.to_string_lossy().contains(':') => {
                return Err(format!("invalid path: {raw}"))
            }
            _ => {}
        }
    }
    // `C:foo` is relative to that drive's current directory, not to the workspace.
    if !given.is_absolute() && matches!(given.components().next(), Some(Component::Prefix(_))) {
        return Err(format!("drive-relative paths are not allowed: {raw}"));
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
                // Only a name with no directory entry may be created later. An entry that exists
                // but does not resolve (a dangling symlink or junction) would be followed on write.
                match std::fs::symlink_metadata(&existing) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    _ => return Err(format!("{raw} cannot be resolved inside the workspace")),
                }
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
