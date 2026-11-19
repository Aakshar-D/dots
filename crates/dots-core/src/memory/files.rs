//! A project's memory as files (memory spec section 4): one small Markdown file for each fact
//! in `<project folder>/.sage/memory/`, and `MEMORY.md`, a list of them that Sage rewrites.
//! The files are the truth: a person may add, change or delete them by hand, so every read
//! takes them as they are now, and a file that cannot be read as a memory is left out and
//! counted, never an error. Sage names the files itself and reaches a file only by a name the
//! folder lists, so no id, text or path a model gives can name a file outside the folder.
//!
//! Everything here blocks on the disk: `Memories` calls it from a blocking task, one writer at
//! a time.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

use super::{looks_secret, MAX_TEXT};
use crate::model::{Memory, MemoryKind, MemoryScope, MemorySource};
use crate::util::timestamp;

/// The folders under a project's folder that hold its memory.
pub const DIR: [&str; 2] = [".sage", "memory"];
/// The list of the project's memories, which Sage rewrites.
pub const INDEX_FILE: &str = "MEMORY.md";
/// The most memories Sage writes for a project. Files added by hand beyond it are still read.
pub const MAX_PER_PROJECT: usize = 300;
/// A larger file is not read as a memory.
pub const MAX_FILE_BYTES: u64 = 16 * 1024;
/// The most files of the folder one read looks at.
pub const MAX_SCANNED: usize = 1000;
/// The most project files a memory names, and the longest such path, in bytes.
pub const MAX_FILES: usize = 8;
pub const MAX_FILE_PATH: usize = 200;
/// The refusal of a list of files with a path outside the project, or too many.
pub const FILES_RULE: &str =
    "A memory names up to 8 files, each a path inside the project such as src/main.rs.";
/// The refusal when the memory folder leads out of the project (a link).
pub const OUTSIDE: &str = "The project's memory folder is not inside the project.";

/// How many characters of a memory's text its line in `MEMORY.md` shows.
const INDEX_TITLE_CHARS: usize = 100;

/// The folder a project's memory files are in.
pub fn dir_of(root: &Path) -> PathBuf {
    DIR.iter()
        .fold(root.to_path_buf(), |dir, part| dir.join(part))
}

/// One memory of a project and the file it is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fact {
    pub memory: Memory,
    pub file_name: String,
}

/// What a read of a project's memory folder found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Loaded {
    /// Pinned first, then newest.
    pub facts: Vec<Fact>,
    /// The files that are no memory: too large, not text, no text in them, a link, a secret.
    pub skipped: usize,
}

impl Loaded {
    pub fn memories(&self) -> Vec<Memory> {
        self.facts.iter().map(|f| f.memory.clone()).collect()
    }

    pub fn find(&self, id: &str) -> Option<&Fact> {
        self.facts.iter().find(|f| f.memory.id == id)
    }
}

/// A project file a memory is about, as it is kept: relative to the project's folder, with
/// `/`, without `./` in front or `/` behind. `None` for a path that is not inside the project:
/// an absolute one, one with a drive or a `..`, one too long, or one with a control character.
pub fn clean_file(raw: &str) -> Option<String> {
    let mut path = raw.trim().replace('\\', "/");
    while let Some(rest) = path.strip_prefix("./") {
        path = rest.to_string();
    }
    let path = path.trim_end_matches('/');
    if path.is_empty()
        || path.len() > MAX_FILE_PATH
        || path.starts_with('/')
        || path.starts_with('~')
        || path.contains(':')
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part.trim() != part)
    {
        return None;
    }
    Some(path.to_string())
}

/// The files of a memory as they are kept (`clean_file`), each once. `Err(FILES_RULE)` when
/// one is not inside the project or there are more than `MAX_FILES`.
pub fn clean_files(raw: &[String]) -> Result<Vec<String>, &'static str> {
    let mut out: Vec<String> = Vec::new();
    for file in raw {
        let file = clean_file(file).ok_or(FILES_RULE)?;
        if !out.contains(&file) {
            out.push(file);
        }
    }
    if out.len() > MAX_FILES {
        return Err(FILES_RULE);
    }
    Ok(out)
}

/// The files a hand-written list names: those that are inside the project, the first
/// `MAX_FILES` of them.
fn lenient_files(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for file in raw.iter().filter_map(|f| clean_file(f)) {
        if !out.contains(&file) && out.len() < MAX_FILES {
            out.push(file);
        }
    }
    out
}

/// Whether `name` is a plain file name of the memory folder: no folder in it, no drive, and
/// `.md` at its end.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.contains(['/', '\\', ':'])
        && name.to_ascii_lowercase().ends_with(".md")
        && !name.eq_ignore_ascii_case(INDEX_FILE)
}

/// An id a memory file may carry: letters, digits, `-` and `_`, 4 to 64 of them.
fn valid_id(id: &str) -> bool {
    (4..=64).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The id of a file that names none of its own (or one another file has): `f` and 15 hex
/// digits of the file's name, so it stays the same while the file keeps its name.
fn derived_id(file_name: &str) -> String {
    let digest = Sha256::digest(file_name.to_lowercase().as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    format!("f{}", &hex[..15])
}

/// `raw` as the store writes a time, when it is an RFC 3339 time.
fn time_of(raw: &str) -> Option<String> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|t| timestamp(t.with_timezone(&Utc)))
}

/// A frontmatter value without the quotes around it.
fn unquoted(value: &str) -> &str {
    let v = value.trim();
    for q in ['"', '\''] {
        if v.len() >= 2 && v.starts_with(q) && v.ends_with(q) {
            return &v[1..v.len() - 1];
        }
    }
    v
}

/// What a memory file's frontmatter says, each value as text.
#[derive(Default)]
struct Meta {
    id: Option<String>,
    kind: Option<String>,
    created: Option<String>,
    updated: Option<String>,
    source: Option<String>,
    run: Option<String>,
    pinned: Option<String>,
    files: Vec<String>,
}

/// The frontmatter and the text of a memory file's `content`. A file without frontmatter is
/// all text. `None` when the frontmatter opens and never closes.
fn split(content: &str) -> Option<(Meta, String)> {
    let content = content.trim_start_matches('\u{feff}');
    let mut lines = content.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Some((Meta::default(), content.to_string()));
    }
    let mut meta = Meta::default();
    let mut in_files = false;
    let mut closed = false;
    let mut body = Vec::new();
    for line in lines {
        if closed {
            body.push(line);
            continue;
        }
        if matches!(line.trim_end(), "---" | "...") {
            closed = true;
            continue;
        }
        if in_files {
            if let Some(item) = line.trim_start().strip_prefix("- ") {
                meta.files.push(unquoted(item).to_string());
                continue;
            }
            in_files = false;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = unquoted(value).to_string();
        match key.trim() {
            "id" => meta.id = Some(value),
            "kind" => meta.kind = Some(value),
            "created" => meta.created = Some(value),
            "updated" => meta.updated = Some(value),
            "source" => meta.source = Some(value),
            "run" => meta.run = Some(value),
            "pinned" => meta.pinned = Some(value),
            "files" => {
                // `files: [a, b]` on one line, or a list on the lines below.
                let inline = value.trim_start_matches('[').trim_end_matches(']');
                if inline.trim().is_empty() {
                    in_files = true;
                } else {
                    meta.files
                        .extend(inline.split(',').map(|f| unquoted(f).to_string()));
                }
            }
            _ => {}
        }
    }
    closed.then(|| (meta, body.join("\n")))
}

/// The memory in a file named `file_name` with `content`, last changed at `modified`. What the
/// frontmatter leaves out or gets wrong is filled in: the kind is `semantic`, the writer the
/// person, the times the file's, and the id comes from the file's name (also when `seen` has
/// the id already). A text longer than a memory is read as its first 500 characters. `None`
/// for a file with no text, an open frontmatter or a secret.
pub fn parse(
    file_name: &str,
    content: &str,
    modified: Option<DateTime<Utc>>,
    seen: &mut HashSet<String>,
) -> Option<Memory> {
    let (meta, body) = split(content)?;
    let text: String = body.trim().chars().take(MAX_TEXT).collect();
    let text = text.trim().to_string();
    if text.is_empty() || looks_secret(&text) {
        return None;
    }
    let id = match meta.id {
        Some(id) if valid_id(&id) && !seen.contains(&id) => id,
        _ => derived_id(file_name),
    };
    if !seen.insert(id.clone()) {
        return None;
    }
    let file_time = modified.map(timestamp);
    let created = meta
        .created
        .as_deref()
        .and_then(time_of)
        .or_else(|| file_time.clone())
        .unwrap_or_else(|| timestamp(DateTime::<Utc>::UNIX_EPOCH));
    let updated = meta
        .updated
        .as_deref()
        .and_then(time_of)
        .or(file_time)
        .unwrap_or_else(|| created.clone());
    let source = match meta
        .source
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("sol" | "dot") => MemorySource::Dot,
        Some("capture") => MemorySource::Capture,
        _ => MemorySource::Person,
    };
    Some(Memory {
        id,
        dot_id: String::new(),
        text,
        pinned: matches!(
            meta.pinned
                .as_deref()
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("true" | "yes")
        ),
        source,
        run_id: meta.run.filter(|run| valid_id(run)),
        created_at: created,
        updated_at: updated,
        kind: meta
            .kind
            .and_then(|k| MemoryKind::parse(&k.to_ascii_lowercase()).ok())
            .unwrap_or(MemoryKind::Semantic),
        scope: MemoryScope::Project,
        files: lenient_files(&meta.files),
    })
}

/// The file of `memory`: its frontmatter and its text.
pub fn render(memory: &Memory) -> String {
    let source = match memory.source {
        MemorySource::Dot => "sol",
        other => other.as_str(),
    };
    let mut out = format!(
        "---\nid: {}\nkind: {}\ncreated: {}\nupdated: {}\nsource: {source}\n",
        memory.id,
        memory.kind.as_str(),
        memory.created_at,
        memory.updated_at,
    );
    if let Some(run) = memory.run_id.as_deref().filter(|run| valid_id(run)) {
        out.push_str(&format!("run: {run}\n"));
    }
    out.push_str(&format!("pinned: {}\n", memory.pinned));
    let files = lenient_files(&memory.files);
    if !files.is_empty() {
        out.push_str("files:\n");
        for file in files {
            out.push_str(&format!("  - {file}\n"));
        }
    }
    out.push_str("---\n");
    out.push_str(memory.text.trim());
    out.push('\n');
    out
}

/// The first words of `text` as a file name's start: ASCII letters and digits in lower case,
/// joined by `-`, at most 40 characters. Empty when the text has none.
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for word in text
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(6)
    {
        if out.len() + word.len() + 1 > 40 {
            break;
        }
        if !out.is_empty() {
            out.push('-');
        }
        out.push_str(&word.to_ascii_lowercase());
    }
    out
}

/// A new file's name for `memory` in `dir`: the slug of its text (its kind when the text gives
/// none), the end of its id, and a number when that name is taken.
fn new_name(dir: &Path, memory: &Memory) -> String {
    let mut stem = slug(&memory.text);
    if stem.is_empty() {
        stem = memory.kind.as_str().to_string();
    }
    let base = format!("{stem}-{}", id_tail(&memory.id));
    let mut name = format!("{base}.md");
    let mut n = 2;
    while dir.join(&name).exists() {
        name = format!("{base}-{n}.md");
        n += 1;
    }
    name
}

/// The end of a file's name that `new_name` takes from `id`: its last 8 letters and digits, in
/// lower case.
fn id_tail(id: &str) -> String {
    let id: String = id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    id[id.len().saturating_sub(8)..].to_string()
}

/// Whether `file_name` is a name Sage gives the file of the memory `id` (`new_name`): it ends
/// in the end of the id, or in that and a number.
fn named_for(file_name: &str, id: &str) -> bool {
    let name = file_name.to_ascii_lowercase();
    let stem = name.strip_suffix(".md").unwrap_or(&name);
    let tail = format!("-{}", id_tail(id));
    stem.ends_with(&tail)
        || stem.rsplit_once('-').is_some_and(|(before, n)| {
            before.ends_with(&tail) && !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())
        })
}

/// The id a memory file's frontmatter carries, when it is one a memory may have.
fn claimed_id(content: &str) -> Option<String> {
    split(content)?.0.id.filter(|id| valid_id(id))
}

/// What a read of a project's memory folder depends on: the entries `load` looks at, each
/// with whether it is a plain file, its size and when it was changed, as the folder lists
/// them, without opening one. While it stays the same, `load` finds the same memories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp(Vec<(String, bool, u64, Option<std::time::SystemTime>)>);

/// The stamp of the project's memory folder as it is now. `None` for a project with no
/// memory folder, or one whose folder leads out of the project: `load` finds nothing there.
pub fn stamp(root: &Path) -> Option<Stamp> {
    let dir = inside(root).ok()?;
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().into_string().ok()?;
            if !plain_name(&name) {
                return None;
            }
            // As the folder lists it: a link is not followed.
            let meta = entry.metadata().ok();
            Some((
                name,
                meta.as_ref().is_some_and(|m| m.is_file()),
                meta.as_ref().map_or(0, |m| m.len()),
                meta.and_then(|m| m.modified().ok()),
            ))
        })
        .collect();
    entries.sort();
    Some(Stamp(entries))
}

/// `path` without Windows' `\\?\` in front, so two canonical paths compare.
fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    let c = std::fs::canonicalize(path)?;
    let text = c.to_string_lossy();
    Ok(match text.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => c,
    })
}

/// The project's memory folder when it exists and is inside the project (a link that leads
/// out of the project is refused).
fn inside(root: &Path) -> std::io::Result<PathBuf> {
    let dir = dir_of(root);
    let (root, real) = (canonical(root)?, canonical(&dir)?);
    if real.starts_with(&root) && real.is_dir() {
        Ok(dir)
    } else {
        Err(std::io::Error::other(OUTSIDE))
    }
}

/// The project's memory folder, made when missing.
pub fn ensure_dir(root: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir_of(root))?;
    inside(root)
}

/// Reads the project's memories from its files, as they are now. Nothing for a project with
/// no memory folder, or one whose folder leads out of the project.
pub fn load(root: &Path) -> Loaded {
    let Ok(dir) = inside(root) else {
        return Loaded::default();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Loaded::default();
    };
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|name| plain_name(name))
        .collect();
    names.sort();
    let mut out = Loaded::default();
    out.skipped += names.len().saturating_sub(MAX_SCANNED);
    names.truncate(MAX_SCANNED);
    // (the file's name, its text, when it was changed, whether Sage named it for its id)
    let mut read = Vec::new();
    for name in names {
        let path = dir.join(&name);
        // A link is not followed: it could name a file anywhere.
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() && meta.len() <= MAX_FILE_BYTES => meta,
            _ => {
                out.skipped += 1;
                continue;
            }
        };
        let modified = meta.modified().ok().map(DateTime::<Utc>::from);
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                let own = claimed_id(&content).is_some_and(|id| named_for(&name, &id));
                read.push((name, content, modified, own));
            }
            Err(_) => out.skipped += 1,
        }
    }
    // When two files carry one id, the file Sage named for that id keeps it, whatever the
    // other is called: a copy made by hand never takes a memory's id from it.
    read.sort_by_key(|(_, _, _, own)| !own);
    let mut seen = HashSet::new();
    for (name, content, modified, _) in read {
        match parse(&name, &content, modified, &mut seen) {
            Some(memory) => out.facts.push(Fact {
                memory,
                file_name: name,
            }),
            None => out.skipped += 1,
        }
    }
    out.facts.sort_by(|a, b| {
        let (a, b) = (&a.memory, &b.memory);
        b.pinned
            .cmp(&a.pinned)
            .then_with(|| b.created_at.cmp(&a.created_at))
            .then_with(|| b.id.cmp(&a.id))
    });
    out
}

/// Writes `content` to the file `name` of `dir` in one step: a new file beside it, then
/// renamed over it, so a reader never finds half a file.
fn write_atomic(dir: &Path, name: &str, content: &str) -> std::io::Result<()> {
    let tmp = dir.join(format!(".{}.tmp", crate::util::new_id()));
    let written = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, dir.join(name))
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

fn not_a_name(name: &str) -> std::io::Error {
    std::io::Error::other(format!("{name:?} is not a memory file's name"))
}

/// Writes a new memory of the project to a file of its own; the file's name.
pub fn save_new(root: &Path, memory: &Memory) -> std::io::Result<String> {
    let dir = ensure_dir(root)?;
    let name = new_name(&dir, memory);
    if !plain_name(&name) {
        return Err(not_a_name(&name));
    }
    write_atomic(&dir, &name, &render(memory))?;
    Ok(name)
}

/// Writes `memory` over its file `file_name`, a name `load` gave.
pub fn rewrite(root: &Path, file_name: &str, memory: &Memory) -> std::io::Result<()> {
    if !plain_name(file_name) {
        return Err(not_a_name(file_name));
    }
    let dir = inside(root)?;
    write_atomic(&dir, file_name, &render(memory))
}

/// Deletes the memory file `file_name`, a name `load` gave. A file already gone is deleted.
pub fn remove(root: &Path, file_name: &str) -> std::io::Result<()> {
    if !plain_name(file_name) {
        return Err(not_a_name(file_name));
    }
    let dir = inside(root)?;
    match std::fs::remove_file(dir.join(file_name)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// `text` on one line, cut to `max` characters with `…` when longer.
pub fn one_line(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max {
        return line;
    }
    let mut out: String = line.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The text of `MEMORY.md` for `facts`: one line for each, by kind.
pub fn index_text(facts: &[Fact]) -> String {
    let mut out = String::from(
        "# Project memory\n\n\
         Sage keeps this list: one line for each fact, by kind. A fact's text is in its own \
         file in this folder. Add, edit or delete those files as you like; Sage reads them \
         again and rewrites this list.\n",
    );
    for (kind, title) in [
        (MemoryKind::Core, "Core"),
        (MemoryKind::Semantic, "Facts and decisions"),
        (MemoryKind::Procedural, "How-to"),
        (MemoryKind::Episodic, "What runs did"),
    ] {
        let of_kind: Vec<&Fact> = facts.iter().filter(|f| f.memory.kind == kind).collect();
        if of_kind.is_empty() {
            continue;
        }
        out.push_str(&format!("\n## {title}\n\n"));
        for fact in of_kind {
            out.push_str(&format!(
                "- [{}] {} ({})\n",
                fact.memory.id,
                one_line(&fact.memory.text, INDEX_TITLE_CHARS),
                fact.file_name
            ));
        }
    }
    out
}

/// Rewrites `MEMORY.md` from the project's files as they are now, when it differs. Nothing
/// for a project without a memory folder.
pub fn write_index(root: &Path) -> std::io::Result<()> {
    let Ok(dir) = inside(root) else {
        return Ok(());
    };
    let text = index_text(&load(root).facts);
    if std::fs::read_to_string(dir.join(INDEX_FILE)).is_ok_and(|now| now == text) {
        return Ok(());
    }
    write_atomic(&dir, INDEX_FILE, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_path_is_kept_only_inside_the_project() {
        assert_eq!(clean_file(" ./src\\main.rs "), Some("src/main.rs".into()));
        assert_eq!(
            clean_file("app/[slug]/page.tsx"),
            Some("app/[slug]/page.tsx".into())
        );
        assert_eq!(clean_file("docs/"), Some("docs".into()));
        for bad in [
            "",
            "/etc/passwd",
            "C:/Windows/system.ini",
            "c:\\x",
            "\\\\server\\share\\x",
            "../outside.rs",
            "src/../../outside.rs",
            "src/./x",
            "~/x",
            "a//b",
            "a\nb",
            "x:stream",
            " a/ b",
        ] {
            assert_eq!(clean_file(bad), None, "{bad:?}");
        }
        assert_eq!(clean_file(&"a".repeat(MAX_FILE_PATH + 1)), None);
    }

    #[test]
    fn a_slug_is_plain_lower_case_words() {
        assert_eq!(slug("Use pnpm, not npm!"), "use-pnpm-not-npm");
        assert_eq!(slug("../../etc/passwd"), "etc-passwd");
        assert_eq!(slug("CON"), "con");
        assert_eq!(slug("日本語"), "");
        assert!(slug(&"word ".repeat(40)).len() <= 40);
    }

    #[test]
    fn a_file_without_frontmatter_is_the_persons_note() {
        let mut seen = HashSet::new();
        let m = parse("note.md", "Deploys go out on Fridays.\n", None, &mut seen).unwrap();
        assert_eq!(m.text, "Deploys go out on Fridays.");
        assert_eq!(m.kind, MemoryKind::Semantic);
        assert_eq!(m.source, MemorySource::Person);
        assert_eq!(m.id, derived_id("note.md"));
        assert!(m.id.len() == 16 && valid_id(&m.id));
        // The same file read again has the id taken: one memory, not two.
        assert_eq!(parse("note.md", "Again.", None, &mut seen), None);
    }

    #[test]
    fn a_written_file_reads_back_as_it_was() {
        let memory = Memory {
            id: "01JABCDEFGHJKMNPQRSTVWXYZ0".into(),
            dot_id: String::new(),
            text: "Run the tests with\ncargo test -p core.".into(),
            pinned: true,
            source: MemorySource::Dot,
            run_id: Some("01JRUN".into()),
            created_at: "2026-11-19T10:00:00.000Z".into(),
            updated_at: "2026-11-19T11:00:00.000Z".into(),
            kind: MemoryKind::Procedural,
            scope: MemoryScope::Project,
            files: vec!["app/[id]/page.tsx".into(), "src/a, b.rs".into()],
        };
        let text = render(&memory);
        assert!(text.starts_with("---\nid: 01JABCDEFGHJKMNPQRSTVWXYZ0\nkind: procedural\n"));
        assert!(text
            .contains("\nsource: sol\nrun: 01JRUN\npinned: true\nfiles:\n  - app/[id]/page.tsx\n"));
        let back = parse("x.md", &text, None, &mut HashSet::new()).unwrap();
        assert_eq!(back, memory);
    }

    #[test]
    fn what_the_frontmatter_gets_wrong_is_filled_in() {
        let content = "---\nid: ../../x\nkind: nonsense\ncreated: yesterday\nsource: root\n\
                       run: a b\npinned: maybe\nfiles: [src/a.rs, ../b.rs, \"C:/c.rs\"]\n\
                       extra: 1\n---\n\n  A fact.  \n";
        let m = parse("odd.md", content, None, &mut HashSet::new()).unwrap();
        assert_eq!(m.id, derived_id("odd.md"));
        assert_eq!(m.kind, MemoryKind::Semantic);
        assert_eq!(m.source, MemorySource::Person);
        assert_eq!(m.run_id, None);
        assert!(!m.pinned);
        assert_eq!(m.files, ["src/a.rs"]);
        assert_eq!(m.text, "A fact.");
        assert_eq!(m.created_at, "1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn a_file_with_no_text_an_open_frontmatter_or_a_secret_is_no_memory() {
        let mut seen = HashSet::new();
        assert_eq!(parse("a.md", "", None, &mut seen), None);
        assert_eq!(
            parse("b.md", "---\nid: abcd\n---\n  \n", None, &mut seen),
            None
        );
        assert_eq!(
            parse("c.md", "---\nid: abcd\nA fact.", None, &mut seen),
            None
        );
        assert_eq!(
            parse("d.md", "password: hunter2hunter2", None, &mut seen),
            None
        );
        // A long text is read as a memory's 500 characters.
        let long = parse("e.md", &"x".repeat(900), None, &mut seen).unwrap();
        assert_eq!(long.text.chars().count(), MAX_TEXT);
    }
}
