use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::Value;

use super::{
    cap_head, confine, display, hidden_note, str_arg, PathFilter, ToolOutput, MAX_ENTRIES,
};

const MAX_MATCHES: usize = 200;
const MAX_LINE_CHARS: usize = 300;
/// Files larger than this are skipped by `grep`.
const MAX_GREP_FILE: u64 = 2 * 1024 * 1024;

/// Walks `base`, honouring `.gitignore` files and skipping `.git` and the entries `filter`
/// hides; a hidden directory is not entered. Hidden entries are counted in `hidden`.
fn walk(
    root: &Path,
    base: &Path,
    filter: &PathFilter,
    hidden: &Arc<AtomicUsize>,
) -> impl Iterator<Item = PathBuf> {
    let (root, filter, hidden) = (root.to_path_buf(), filter.clone(), hidden.clone());
    ignore::WalkBuilder::new(base)
        .hidden(false)
        .require_git(false)
        .sort_by_file_path(|a, b| a.cmp(b))
        .filter_entry(move |e| {
            if e.file_name() == ".git" {
                return false;
            }
            // Depth 0 is `base` itself, which the permission gate already judged.
            if e.depth() > 0 && filter.hides(&display(&root, e.path())) {
                hidden.fetch_add(1, Ordering::Relaxed);
                return false;
            }
            true
        })
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

pub(super) async fn glob(root: &Path, input: &Value, filter: &PathFilter) -> ToolOutput {
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
    let (root, filter) = (root.to_path_buf(), filter.clone());
    let found = tokio::task::spawn_blocking(move || {
        let hidden = Arc::new(AtomicUsize::new(0));
        let mut hits: Vec<String> = walk(&root, &base, &filter, &hidden)
            .filter(|p| matcher.is_match(p.strip_prefix(&base).unwrap_or(p)))
            .map(|p| display(&root, &p))
            .collect();
        let note = hidden_note(hidden.load(Ordering::Relaxed));
        if hits.is_empty() {
            return ToolOutput::ok(format!("no files match {pattern}{note}"));
        }
        hits.sort();
        let total = hits.len();
        hits.truncate(MAX_ENTRIES);
        let mut out = hits.join("\n");
        if total > MAX_ENTRIES {
            out.push_str(&format!("\n[{} more files]", total - MAX_ENTRIES));
        }
        ToolOutput::ok(cap_head(out) + &note)
    })
    .await;
    found.unwrap_or_else(|e| ToolOutput::err(format!("glob failed: {e}")))
}

pub(super) async fn grep(root: &Path, input: &Value, filter: &PathFilter) -> ToolOutput {
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
    let only = match input.get("glob").and_then(Value::as_str) {
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
    let (root, filter) = (root.to_path_buf(), filter.clone());
    let found = tokio::task::spawn_blocking(move || {
        let hidden = Arc::new(AtomicUsize::new(0));
        let mut hits = Vec::new();
        'files: for path in walk(&root, &base, &filter, &hidden) {
            let rel = display(&root, &path);
            if only.as_ref().is_some_and(|f| !f.is_match(&rel)) {
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
        let note = hidden_note(hidden.load(Ordering::Relaxed));
        if hits.is_empty() {
            return ToolOutput::ok(format!("no matches for {pattern}{note}"));
        }
        ToolOutput::ok(cap_head(hits.join("\n")) + &note)
    })
    .await;
    found.unwrap_or_else(|e| ToolOutput::err(format!("grep failed: {e}")))
}
