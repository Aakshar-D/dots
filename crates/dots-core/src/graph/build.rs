//! A build of a project's graph (project graph spec section 5): scan, parse what changed,
//! link the whole graph from the stored facts, write it. Blocking.

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Instant, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use super::extract::{extract, Lang};
use super::link::{link, Markers, Parsed};
use super::map;
use super::store::{self, Changed, FileRow, Maps, Meta};

/// A larger file is skipped.
pub const MAX_FILE_BYTES: u64 = 512 * 1024;
/// Files past this many (in path order) are skipped.
pub const MAX_FILES: usize = 20_000;
/// Folders never read, wherever they are.
pub const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    "build",
    "vendor",
    ".git",
    "graphify-out",
    "__pycache__",
    ".venv",
    "venv",
];
/// A file with a NUL byte in this many first bytes is binary.
const BINARY_PROBE: usize = 8 * 1024;

/// What a build did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub files: usize,
    pub parsed: usize,
    pub deleted: usize,
    pub skipped: usize,
    pub nodes: usize,
    pub edges: usize,
    pub duration_ms: u64,
}

/// The limits of a build; tests make them small.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_files: usize,
    pub max_file_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_files: MAX_FILES,
            max_file_bytes: MAX_FILE_BYTES,
        }
    }
}

struct Candidate {
    path: String,
    lang: Lang,
    size: u64,
    mtime: i64,
}

/// What the scan found: the files to read, the markers, and how many were skipped.
struct Scan {
    files: Vec<Candidate>,
    markers: Markers,
    skipped: usize,
}

fn rel_path(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let parts: Vec<String> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The `name` of a `Cargo.toml`'s `[package]`.
fn crate_name(text: &str) -> Option<String> {
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package {
            if let Some(rest) = line.strip_prefix("name") {
                let value = rest.trim_start().strip_prefix('=')?.trim();
                return Some(value.trim_matches('"').to_string());
            }
        }
    }
    None
}

fn scan(root: &Path, limits: Limits) -> Scan {
    let walker = ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .ignore(true)
        .parents(true)
        .require_git(false)
        .follow_links(false)
        .filter_entry(|e| {
            let is_dir = e.file_type().is_some_and(|t| t.is_dir());
            !(is_dir && SKIP_DIRS.iter().any(|d| e.file_name() == *d))
        })
        .build();
    let mut files = Vec::new();
    let mut markers = Markers::default();
    let mut skipped = 0;
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let Some(rel) = rel_path(root, entry.path()) else {
            continue;
        };
        let name = rel.rsplit('/').next().unwrap_or(&rel);
        let dir = rel.rsplit_once('/').map_or("", |(d, _)| d).to_string();
        match name {
            "Cargo.toml" => {
                let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
                if let Some(n) = crate_name(&text) {
                    markers.cargo.insert(dir.clone(), n);
                }
            }
            "package.json" => {
                markers.package_json.insert(dir.clone());
            }
            "go.mod" => {
                let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
                if let Some(m) = text.lines().find_map(|l| l.trim().strip_prefix("module ")) {
                    markers.go_mod.insert(dir.clone(), m.trim().to_string());
                }
            }
            _ => {}
        }
        let Some(lang) = Lang::of(&rel) else {
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.len() > limits.max_file_bytes {
            skipped += 1;
            continue;
        }
        files.push(Candidate {
            path: rel,
            lang,
            size: meta.len(),
            mtime: mtime_ms(&meta),
        });
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    if files.len() > limits.max_files {
        skipped += files.len() - limits.max_files;
        files.truncate(limits.max_files);
    }
    Scan {
        files,
        markers,
        skipped,
    }
}

/// The bytes of the file, or `None` when it cannot be read or is binary.
fn read_text(path: &Path, max: u64) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(max + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > max || bytes.iter().take(BINARY_PROBE).any(|b| *b == 0) {
        return None;
    }
    Some(bytes)
}

fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Builds the graph of `root` into the file `db`, parsing only the files that changed since
/// the last build. `progress` is told (done, total) as files are checked and parsed.
pub fn build(
    root: &Path,
    db: &Path,
    commit: Option<String>,
    limits: Limits,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Report, String> {
    let started = Instant::now();
    let mut conn = store::open(db).map_err(|e| format!("the graph file cannot be opened: {e}"))?;
    let rows = store::file_rows(&conn).map_err(|e| format!("reading the graph failed: {e}"))?;
    let scan = scan(root, limits);
    let mut skipped = scan.skipped;

    // Which files changed: size and time first, then the hash.
    struct Todo {
        cand_index: usize,
        bytes: Vec<u8>,
        hash: String,
    }
    let total_check = scan.files.len();
    let mut todo: Vec<Todo> = Vec::new();
    let mut touched: Vec<Changed> = Vec::new();
    let mut unreadable: HashSet<&str> = HashSet::new();
    for (i, c) in scan.files.iter().enumerate() {
        if i % 200 == 0 {
            progress(i, total_check * 2);
        }
        let row = rows.get(&c.path);
        if row.is_some_and(|r| r.size == c.size && r.mtime == c.mtime) {
            continue;
        }
        let Some(bytes) = read_text(&root.join(&c.path), limits.max_file_bytes) else {
            skipped += 1;
            unreadable.insert(c.path.as_str());
            continue;
        };
        let h = hash(&bytes);
        if row.is_some_and(|r| r.hash == h) {
            touched.push(Changed {
                path: c.path.clone(),
                row: FileRow {
                    size: c.size,
                    mtime: c.mtime,
                    hash: h,
                },
                lang: c.lang,
                facts: None,
            });
            continue;
        }
        todo.push(Todo {
            cand_index: i,
            bytes,
            hash: h,
        });
    }

    // Parse the changed files on a few threads.
    let done = AtomicUsize::new(0);
    let to_parse = todo.len();
    let total = total_check + to_parse;
    progress(total_check, total);
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 4);
    let chunk = to_parse.div_ceil(threads).max(1);
    let parsed: Vec<Changed> = std::thread::scope(|s| {
        let handles: Vec<_> = todo
            .chunks(chunk)
            .map(|part| {
                let files = &scan.files;
                let done = &done;
                s.spawn(move || {
                    part.iter()
                        .map(|t| {
                            let c = &files[t.cand_index];
                            let text = String::from_utf8_lossy(&t.bytes);
                            let facts = extract(c.lang, &text).unwrap_or_default();
                            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                            if n.is_multiple_of(50) {
                                progress(total_check + n, total);
                            }
                            Changed {
                                path: c.path.clone(),
                                row: FileRow {
                                    size: c.size,
                                    mtime: c.mtime,
                                    hash: t.hash.clone(),
                                },
                                lang: c.lang,
                                facts: Some(facts),
                            }
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    });

    let present: HashSet<&str> = scan
        .files
        .iter()
        .map(|c| c.path.as_str())
        .filter(|p| !unreadable.contains(p))
        .collect();
    // Nothing changed since a good build: the graph stands as it is.
    let unchanged = parsed.is_empty()
        && !rows.keys().any(|p| !present.contains(p.as_str()))
        && store::read_markers(&conn).as_ref() == Some(&scan.markers);
    if unchanged {
        if let Some(meta) =
            store::read_meta(&conn).filter(|m| m.built_at.is_some() && m.error.is_none())
        {
            store::touch_files(&mut conn, &touched)
                .map_err(|e| format!("writing the graph failed: {e}"))?;
            progress(total, total);
            return Ok(Report {
                files: meta.files as usize,
                parsed: 0,
                deleted: 0,
                skipped,
                nodes: meta.nodes as usize,
                edges: meta.edges as usize,
                duration_ms: started.elapsed().as_millis() as u64,
            });
        }
    }

    // Every file's facts: the new ones, the rest as stored.
    let deleted: Vec<String> = rows
        .keys()
        .filter(|p| !present.contains(p.as_str()))
        .cloned()
        .collect();
    let fresh_paths: HashSet<String> = parsed.iter().map(|c| c.path.clone()).collect();
    let fresh: HashSet<&str> = fresh_paths.iter().map(String::as_str).collect();
    let mut all =
        store::stored_facts(&conn, &fresh).map_err(|e| format!("reading the graph failed: {e}"))?;
    all.retain(|p, _| present.contains(p.as_str()));
    for c in &parsed {
        if let Some(facts) = &c.facts {
            all.insert(
                c.path.clone(),
                Parsed {
                    lang: c.lang,
                    facts: facts.clone(),
                },
            );
        }
    }

    let mut graph = link(&all, &scan.markers);
    super::community::assign(&mut graph);
    let mut langs: BTreeMap<String, usize> = BTreeMap::new();
    for p in all.values() {
        *langs.entry(p.lang.name().to_string()).or_insert(0) += 1;
    }
    let maps = Maps {
        prompt: map::render(&graph, &langs, map::PROMPT),
        full: map::render(&graph, &langs, map::FULL),
    };
    let duration_ms = started.elapsed().as_millis() as u64;
    let meta = Meta {
        root: root.to_string_lossy().into_owned(),
        built_at: Some(crate::util::now()),
        duration_ms,
        files: all.len() as u64,
        skipped: skipped as u64,
        nodes: graph.nodes.len() as u64,
        edges: graph.edges.len() as u64,
        langs,
        commit,
        error: None,
    };
    let mut changed = parsed;
    changed.extend(touched);
    store::write_build(
        &mut conn,
        &changed,
        &deleted,
        &graph,
        &maps,
        &scan.markers,
        &meta,
    )
    .map_err(|e| format!("writing the graph failed: {e}"))?;
    progress(total, total);
    Ok(Report {
        files: all.len(),
        parsed: fresh.len(),
        deleted: deleted.len(),
        skipped,
        nodes: graph.nodes.len(),
        edges: graph.edges.len(),
        duration_ms,
    })
}

/// The changed files of the last build, for tests: (path → hash).
#[cfg(test)]
pub fn hashes(db: &Path) -> std::collections::HashMap<String, String> {
    let conn = store::open(db).unwrap();
    store::file_rows(&conn)
        .unwrap()
        .into_iter()
        .map(|(p, r)| (p, r.hash))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    fn quiet(_: usize, _: usize) {}

    fn ids(db: &Path) -> Vec<String> {
        let conn = store::open(db).unwrap();
        let (g, _, _) = store::load(&conn).unwrap().unwrap();
        g.nodes.into_iter().map(|n| n.id).collect()
    }

    #[test]
    fn a_rebuild_parses_only_what_changed_and_drops_deleted_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        let db = tmp.path().join("g.sqlite");
        write(&root, "src/a.rs", "pub fn a() { b(); }\n");
        write(&root, "src/b.rs", "pub fn b() {}\n");
        write(&root, "src/c.rs", "pub fn c() {}\n");
        write(&root, "node_modules/x/index.js", "function hidden() {}\n");
        write(&root, ".gitignore", "ignored/\n");
        write(&root, "ignored/z.rs", "fn z() {}\n");
        write(&root, "big.rs", &"// x\n".repeat(100));
        let limits = Limits {
            max_files: 100,
            max_file_bytes: 200,
        };

        let first = build(&root, &db, None, limits, &quiet).unwrap();
        assert_eq!(first.files, 3);
        assert_eq!(first.parsed, 3);
        assert_eq!(first.skipped, 1, "big.rs is over the size");
        let all = ids(&db);
        assert!(all.contains(&"src/a.rs#a".to_string()));
        assert!(!all
            .iter()
            .any(|i| i.contains("hidden") || i.contains("z.rs")));

        // Nothing changed: nothing is parsed.
        let again = build(&root, &db, None, limits, &quiet).unwrap();
        assert_eq!(again.parsed, 0);
        assert_eq!(again.nodes, first.nodes);

        // Change one, add one, delete one.
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(&root, "src/a.rs", "pub fn a2() { d(); }\n");
        write(&root, "src/d.rs", "pub fn d() {}\n");
        std::fs::remove_file(root.join("src/c.rs")).unwrap();
        let third = build(&root, &db, None, limits, &quiet).unwrap();
        assert_eq!(third.parsed, 2);
        assert_eq!(third.deleted, 1);
        let all = ids(&db);
        assert!(all.contains(&"src/a.rs#a2".to_string()));
        assert!(all.contains(&"src/d.rs#d".to_string()));
        assert!(!all.contains(&"src/a.rs#a".to_string()));
        assert!(!all.iter().any(|i| i.contains("c.rs")));
        assert_eq!(hashes(&db).len(), 3);

        // A file whose time changed but not its text is not parsed again.
        std::thread::sleep(std::time::Duration::from_millis(20));
        write(&root, "src/b.rs", "pub fn b() {}\n");
        let fourth = build(&root, &db, None, limits, &quiet).unwrap();
        assert_eq!(fourth.parsed, 0);
    }

    #[test]
    fn the_file_cap_skips_files_past_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        for i in 0..5 {
            write(&root, &format!("f{i}.py"), "def f(): pass\n");
        }
        let limits = Limits {
            max_files: 3,
            max_file_bytes: 1000,
        };
        let r = build(&root, &tmp.path().join("g.sqlite"), None, limits, &quiet).unwrap();
        assert_eq!((r.files, r.skipped), (3, 2));
    }

    #[test]
    fn crate_names_come_from_the_package_section() {
        assert_eq!(
            crate_name("[workspace]\nname = \"x\"\n[package]\nname = \"dots-core\"\n"),
            Some("dots-core".into())
        );
        assert_eq!(crate_name("[workspace]\nmembers = []\n"), None);
    }
}
