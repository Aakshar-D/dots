use std::path::Path;

use serde_json::Value;

use super::{cap_head, confine, display, str_arg, ToolOutput, MAX_ENTRIES, MAX_READ_BYTES};

/// Files whose first 8 KB contain a NUL byte are treated as binary and not returned.
fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8192).any(|b| *b == 0)
}

/// The tools run inside the always-on host process, so a huge file is refused, not loaded.
fn too_large(raw: &str, len: u64) -> ToolOutput {
    ToolOutput::err(format!(
        "{raw} is too large ({len} bytes; limit {MAX_READ_BYTES}). Use grep or shell to inspect it."
    ))
}

pub(super) async fn read_file(root: &Path, input: &Value) -> ToolOutput {
    let raw = str_arg(input, "file_path");
    let path = match confine(root, raw) {
        Ok(p) => p,
        Err(e) => return ToolOutput::err(e),
    };
    match tokio::fs::metadata(&path).await {
        Ok(m) if m.is_dir() => {
            return ToolOutput::err(format!("{raw} is a directory; use list_dir"))
        }
        Ok(m) if m.len() > MAX_READ_BYTES => return too_large(raw, m.len()),
        Ok(_) => {}
        Err(e) => return ToolOutput::err(format!("cannot read {raw}: {e}")),
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
    match tokio::fs::metadata(&path).await {
        Ok(m) if m.len() > MAX_READ_BYTES => return too_large(raw, m.len()),
        Ok(_) => {}
        Err(e) => return ToolOutput::err(format!("cannot read {raw}: {e}")),
    }
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
        ToolOutput::ok(cap_head(out))
    })
    .await;
    listed.unwrap_or_else(|e| ToolOutput::err(format!("list_dir failed: {e}")))
}
