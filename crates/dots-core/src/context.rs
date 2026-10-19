//! The documents of a context collection (context library spec section 3): text files below a
//! folder the person chose in the Library. The `context` tool lists them, searches them and
//! reads one, the same way on both engines; it never writes. Each call walks the folder
//! afresh, so an edit shows at the next call.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use crate::kit::parse_frontmatter;

/// The extensions of a document, compared without regard to case.
pub const EXTENSIONS: &[&str] = &[
    "md", "markdown", "txt", "rst", "adoc", "org", "json", "yaml", "yml", "toml", "csv",
];
/// The most documents a collection lists and searches.
pub const MAX_DOCUMENTS: usize = 2_000;
/// A larger document is not read.
pub const MAX_DOCUMENT_BYTES: u64 = 10 * 1024 * 1024;
/// A search stops after reading this much.
pub const MAX_SEARCH_BYTES: u64 = 64 * 1024 * 1024;
/// A tool result is capped at this many bytes.
pub const MAX_OUTPUT: usize = 30 * 1024;
/// The walk stops after visiting this many directory entries, files and folders together.
pub const MAX_WALK_ENTRIES: usize = 100_000;
/// The most collections the prompt lists.
pub const MAX_PROMPT_COLLECTIONS: usize = 50;
/// The longest summary and matching line shown, in characters.
const MAX_LINE: usize = 200;
const MAX_RESULTS: usize = 10;
const MAX_TERMS: usize = 10;
const LINES_PER_RESULT: usize = 3;
/// How much of a document the list reads for its title and summary.
const HEAD_BYTES: u64 = 8 * 1024;

/// What the tool tells the model it does.
pub const DESCRIPTION: &str = "Read the shared context the person keeps for dots: list a \
    collection's documents, search them with `query`, or read one with `path`.";

/// The arguments of the `context` tool, as JSON Schema properties; only `collection` is
/// required.
pub fn properties() -> Value {
    let s = |d: &str| json!({"type": "string", "description": d});
    json!({
        "collection": s("The collection's name, as the prompt lists it"),
        "query": s("Words to search for; a document must have every word"),
        "path": s("A document to read, as the list or a search names it"),
        "offset": {"type": "integer", "description": "With path: the first line to return, 1-based"}
    })
}

/// A collection of a run: its Library name and folder, with the count of documents found when
/// the run opened its tools.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection {
    pub name: String,
    pub dir: PathBuf,
    pub description: String,
    pub documents: usize,
    /// False when the walk stopped at `MAX_WALK_ENTRIES`: `documents` is a lower bound.
    pub complete: bool,
}

/// A document as the list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Document {
    /// Relative to the folder, with `/`.
    pub path: String,
    pub title: String,
    pub summary: String,
}

/// The documents below a folder: the first `MAX_DOCUMENTS` paths in sorted order, and how many
/// there are. `complete` is false when the walk stopped at `MAX_WALK_ENTRIES`: `total` is then
/// a lower bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Walk {
    pub paths: Vec<String>,
    pub total: usize,
    pub complete: bool,
}

/// Whether `name` (a file name) has a document's extension.
fn is_document_name(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// The documents below `root`. Names starting with `.` and links are left out, files and
/// folders alike. The walk visits at most `MAX_WALK_ENTRIES` entries. `Err` when `root` cannot
/// be read as a folder.
pub fn walk(root: &Path) -> Result<Walk, String> {
    walk_bounded(root, MAX_WALK_ENTRIES)
}

/// `walk` with its own bound on the entries visited.
pub fn walk_bounded(root: &Path, max_entries: usize) -> Result<Walk, String> {
    // Only the smallest `MAX_DOCUMENTS` paths are kept, so memory stays bounded.
    let mut kept = BTreeSet::new();
    let mut total = 0;
    let mut visited = 0;
    let mut complete = true;
    let mut folders = vec![(root.to_path_buf(), String::new())];
    let mut first = true;
    'walk: while let Some((folder, prefix)) = folders.pop() {
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(e) if first => return Err(format!("the folder cannot be read: {e}")),
            Err(_) => continue,
        };
        first = false;
        for entry in entries.flatten() {
            visited += 1;
            if visited > max_entries {
                complete = false;
                break 'walk;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if name.starts_with('.') || kind.is_symlink() {
                continue;
            }
            let rel = format!("{prefix}{name}");
            if kind.is_dir() {
                folders.push((entry.path(), format!("{rel}/")));
            } else if kind.is_file() && is_document_name(&name) {
                total += 1;
                kept.insert(rel);
                if kept.len() > MAX_DOCUMENTS {
                    kept.pop_last();
                }
            }
        }
    }
    Ok(Walk {
        paths: kept.into_iter().collect(),
        total,
        complete,
    })
}

/// `text` cut at `max` characters, with `…` when it was cut.
fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// The title and summary of the document at `path` (relative, with `/`) whose text is `text`:
/// the frontmatter's `title`, else the first `# ` heading, else the file name without its
/// extension; the frontmatter's `description`, else the first non-empty line that is not a
/// heading, cut at 200 characters.
pub fn describe(path: &str, text: &str) -> Document {
    let front = parse_frontmatter(text);
    let field = |key: &str| {
        front
            .fields
            .get(key)
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let lines = || front.body.lines().map(str::trim);
    let title = field("title")
        .or_else(|| {
            lines()
                .find_map(|l| l.strip_prefix("# "))
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
        })
        .unwrap_or_else(|| {
            let name = path.rsplit('/').next().unwrap_or(path);
            Path::new(name)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| name.to_string())
        });
    let summary = field("description")
        .or_else(|| {
            lines()
                .find(|l| !l.is_empty() && !l.starts_with('#') && *l != "---")
                .map(str::to_string)
        })
        .unwrap_or_default();
    Document {
        path: path.to_string(),
        title: clip(&title, MAX_LINE),
        summary: clip(&summary, MAX_LINE),
    }
}

/// The joined path of `rel` (relative, with `/`) below `root`.
fn joined(root: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(root.to_path_buf(), |p, part| p.join(part))
}

/// At most `limit` bytes of the file at `path`, with bytes that are not UTF-8 replaced.
fn read_head(path: &Path, limit: u64) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(limit)
        .read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The documents of `root`, described from the first 8 KB of each, with the walk's total and
/// whether the walk was complete (otherwise the total is a lower bound).
pub fn documents(root: &Path, limit: usize) -> Result<(Vec<Document>, usize, bool), String> {
    documents_bounded(root, limit, MAX_WALK_ENTRIES)
}

/// `documents` with a walk that visits at most `max_entries` entries.
pub fn documents_bounded(
    root: &Path,
    limit: usize,
    max_entries: usize,
) -> Result<(Vec<Document>, usize, bool), String> {
    let walk = walk_bounded(root, max_entries)?;
    Ok((describe_walk(root, &walk, limit), walk.total, walk.complete))
}

/// The first `limit` documents of `walk`, described from their heads.
fn describe_walk(root: &Path, walk: &Walk, limit: usize) -> Vec<Document> {
    walk.paths
        .iter()
        .take(limit)
        .map(|rel| {
            let text = read_head(&joined(root, rel), HEAD_BYTES).unwrap_or_default();
            describe(rel, &text)
        })
        .collect()
}

/// Adds `line` to `out` unless it would pass `MAX_OUTPUT`; false when it did not fit.
fn push_line(out: &mut String, line: &str) -> bool {
    if out.len() + line.len() + 1 > MAX_OUTPUT {
        return false;
    }
    if !out.is_empty() {
        out.push('\n');
    }
    out.push_str(line);
    true
}

/// The list of the collection's documents, one `path — title: summary` line each.
pub fn list(root: &Path) -> Result<String, String> {
    list_bounded(root, MAX_WALK_ENTRIES)
}

/// `list` with its own bound on the entries the walk visits.
pub fn list_bounded(root: &Path, max_entries: usize) -> Result<String, String> {
    let walk = walk_bounded(root, max_entries)?;
    let total = walk.total;
    let complete = walk.complete;
    let docs = describe_walk(root, &walk, MAX_DOCUMENTS);
    if docs.is_empty() && complete {
        return Ok("The collection has no documents.".into());
    }
    let mut out = String::new();
    let mut shown = 0;
    for doc in &docs {
        let line = match doc.summary.as_str() {
            "" => format!("- {} — {}", doc.path, doc.title),
            summary => format!("- {} — {}: {summary}", doc.path, doc.title),
        };
        // Room is kept for the closing lines.
        if out.len() + line.len() + 200 > MAX_OUTPUT {
            break;
        }
        push_line(&mut out, &line);
        shown += 1;
    }
    if shown < docs.len() {
        out.push_str(&format!(
            "\n[{} more documents are not listed; use query to search them]",
            docs.len() - shown
        ));
    }
    if !complete {
        out.push_str(&format!(
            "\n[the folder is too large to walk completely: at least {total} documents; only \
             the first {MAX_DOCUMENTS} found are listed and searched]"
        ));
    } else if total > docs.len() {
        out.push_str(&format!(
            "\n[the collection has {total} documents; only the first {MAX_DOCUMENTS} are \
             listed and searched]"
        ));
    }
    Ok(out)
}

/// The search terms of `query`: lower case, at least 2 characters, each once, at most 10.
fn terms(query: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|t| t.chars().count() >= 2 && seen.insert(t.clone()))
        .take(MAX_TERMS)
        .collect()
}

struct Hit {
    doc: Document,
    count: usize,
    lines: Vec<String>,
}

/// The documents that have every word of `query` in their path, title or text, ranked by the
/// count of hits, then by path, with up to 3 matching lines each.
pub fn search(root: &Path, query: &str) -> Result<String, String> {
    let terms = terms(query);
    if terms.is_empty() {
        return Err("query needs a word of at least 2 characters".into());
    }
    let walk = walk(root)?;
    let mut read: u64 = 0;
    let mut stopped = false;
    let mut hits = Vec::new();
    for rel in &walk.paths {
        let path = joined(root, rel);
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.len() > MAX_DOCUMENT_BYTES {
            continue;
        }
        if read + meta.len() > MAX_SEARCH_BYTES {
            stopped = true;
            break;
        }
        read += meta.len();
        let Ok(text) = read_head(&path, MAX_DOCUMENT_BYTES) else {
            continue;
        };
        let doc = describe(rel, &text);
        let lower = text.to_lowercase();
        let known = format!("{} {}", rel.to_lowercase(), doc.title.to_lowercase());
        if !terms
            .iter()
            .all(|t| lower.contains(t.as_str()) || known.contains(t.as_str()))
        {
            continue;
        }
        let count = terms
            .iter()
            .map(|t| lower.matches(t.as_str()).count() + known.matches(t.as_str()).count())
            .sum();
        let lines = text
            .lines()
            .enumerate()
            .filter(|(_, l)| {
                let l = l.to_lowercase();
                terms.iter().any(|t| l.contains(t.as_str()))
            })
            .take(LINES_PER_RESULT)
            .map(|(i, l)| format!("  line {}: {}", i + 1, clip(l.trim(), MAX_LINE)))
            .collect();
        hits.push(Hit { doc, count, lines });
    }
    hits.sort_by(|a, b| b.count.cmp(&a.count).then(a.doc.path.cmp(&b.doc.path)));
    let query = clip(query, MAX_LINE);
    let mut out = if hits.is_empty() {
        format!("No document matches {query:?}.")
    } else if hits.len() > MAX_RESULTS {
        format!(
            "{} documents match {query:?}; the {MAX_RESULTS} best:",
            hits.len()
        )
    } else {
        format!("{} documents match {query:?}:", hits.len())
    };
    for hit in hits.iter().take(MAX_RESULTS) {
        let mut block = format!("\n{} — {}", hit.doc.path, hit.doc.title);
        for line in &hit.lines {
            block.push('\n');
            block.push_str(line);
        }
        if !push_line(&mut out, &block) {
            break;
        }
    }
    if stopped {
        out.push_str("\n[the search stopped after 64 MB; results may be incomplete]");
    }
    if !walk.complete {
        out.push_str("\n[the folder is too large to walk completely; results may be incomplete]");
    }
    Ok(out)
}

/// The document `rel` of the collection at `root`, from line `offset` (1-based), capped at
/// `MAX_OUTPUT`. It must name a document inside the folder: a path that is absolute, holds
/// `..` or a `:`, leads out of the folder through a link, or is hidden is refused, and so is a
/// file that is not a document.
pub fn read(root: &Path, rel: &str, offset: usize) -> Result<String, String> {
    let shown = clip(rel, MAX_LINE);
    let outside = || format!("{shown} is not inside the collection");
    let given = Path::new(rel);
    let plain = given.components().all(|c| match c {
        Component::Normal(n) => {
            let n = n.to_string_lossy();
            !n.contains(':') && !n.starts_with('.')
        }
        Component::CurDir => true,
        _ => false,
    });
    if !plain || rel.trim().is_empty() {
        return Err(outside());
    }
    let canonical_root =
        std::fs::canonicalize(root).map_err(|e| format!("the folder cannot be read: {e}"))?;
    let path = std::fs::canonicalize(canonical_root.join(given))
        .map_err(|_| format!("{shown} was not found in the collection"))?;
    let Ok(below) = path.strip_prefix(&canonical_root) else {
        return Err(outside());
    };
    // A short name or a link can reach a hidden item that the given path does not name.
    if below
        .components()
        .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
    {
        return Err(outside());
    }
    if path.is_dir() {
        return Err(format!("{shown} is a folder; name a document in it"));
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    if !name.as_deref().is_some_and(is_document_name) {
        return Err(format!("{shown} is not a document of the collection"));
    }
    let size = std::fs::metadata(&path)
        .map_err(|e| format!("{shown} could not be read: {e}"))?
        .len();
    if size > MAX_DOCUMENT_BYTES {
        return Err(format!("{shown} is larger than 10 MB"));
    }
    let text = read_head(&path, MAX_DOCUMENT_BYTES)
        .map_err(|e| format!("{shown} could not be read: {e}"))?;
    let lines: Vec<&str> = text.lines().collect();
    if offset > 1 && offset > lines.len() {
        return Err(format!(
            "{shown} has {} lines; offset {offset} is past the end",
            lines.len()
        ));
    }
    let mut out = String::new();
    let mut next = None;
    for (i, line) in lines.iter().enumerate().skip(offset.saturating_sub(1)) {
        // Room is kept for the closing line.
        if out.len() + line.len() + 80 > MAX_OUTPUT {
            if out.is_empty() {
                // One line longer than a result: its start, and the next call goes past it.
                let mut cut = MAX_OUTPUT - 80;
                while !line.is_char_boundary(cut) {
                    cut -= 1;
                }
                out.push_str(&line[..cut]);
                next = (i + 2 <= lines.len()).then_some(i + 2);
                if next.is_none() {
                    out.push_str("\n[truncated: the rest of this line is not shown]");
                }
            } else {
                next = Some(i + 1);
            }
            break;
        }
        if i + 1 > offset.max(1) {
            out.push('\n');
        }
        out.push_str(line);
    }
    if let Some(next) = next {
        out.push_str(&format!("\n[truncated: continue with offset={next}]"));
    }
    Ok(out)
}

/// Runs a `context` call against `collections`: the list without `query` and `path`, a
/// search with `query`, a document with `path`. Blocking: it reads the folder.
pub fn call(collections: &[Collection], input: &Value) -> Result<String, String> {
    let text = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    let name = text("collection").ok_or("collection is required")?;
    let Some(collection) = collections.iter().find(|c| c.name == name) else {
        let name = clip(name, MAX_LINE);
        let names: Vec<&str> = collections.iter().map(|c| c.name.as_str()).collect();
        return Err(if names.is_empty() {
            format!("unknown collection {name:?}. This run has no collections.")
        } else {
            format!(
                "unknown collection {name:?}. Collections: {}",
                names.join(", ")
            )
        });
    };
    let offset = match input.get("offset") {
        None | Some(Value::Null) => 1,
        Some(v) => match v.as_u64() {
            Some(n) if n >= 1 => usize::try_from(n).unwrap_or(usize::MAX),
            _ => return Err("offset must be a whole number of at least 1".into()),
        },
    };
    if !collection.dir.is_dir() {
        return Err(format!("the folder of collection {name} is missing"));
    }
    match (text("query"), text("path")) {
        (Some(_), Some(_)) => {
            Err("give either query (to search) or path (to read a document), not both".into())
        }
        (Some(query), None) => search(&collection.dir, query),
        (None, Some(path)) => read(&collection.dir, path, offset),
        (None, None) => list(&collection.dir),
    }
}

/// The prompt's section that lists `collections` for the tool named `tool`; empty without
/// collections.
pub fn prompt_section(collections: &[Collection], tool: &str) -> String {
    if collections.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "Shared context you can read. Call the tool `{tool}` with a collection name to list its \
         documents, `query` to search it, or `path` to read one. Look here before you guess \
         how the person works."
    );
    for c in collections.iter().take(MAX_PROMPT_COLLECTIONS) {
        let description = match c.description.trim() {
            "" => "(no description)",
            d => d,
        };
        let count = match (c.documents, c.complete) {
            (1, true) => "1 document".to_string(),
            (n, true) => format!("{n} documents"),
            (1, false) => "at least 1 document".to_string(),
            (n, false) => format!("at least {n} documents"),
        };
        out.push_str(&format!("\n- {}: {description} ({count})", c.name));
    }
    out
}
