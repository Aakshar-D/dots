//! A build of a project's graph (project graph spec section 5): scan, parse what changed,
//! link the whole graph from the stored facts, write it. Blocking.

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, UNIX_EPOCH};

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
/// The walk of a folder stops after this long: a huge folder must not keep a build going.
pub const WALK_BUDGET: Duration = Duration::from_secs(60);
/// The parse threads' stack: tree-sitter and the walker run deep on odd files.
const PARSE_STACK: usize = 32 * 1024 * 1024;
/// A file the test builds make the parser panic on, to check that a panic is counted.
#[cfg(test)]
const PANIC_MARKER: &str = "// dots-graph-test: panic";
/// A file with a NUL byte in this many first bytes is binary.
const BINARY_PROBE: usize = 8 * 1024;

/// What a build did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// Files whose parse failed (the parser panicked); counted in `skipped` too.
    pub parse_errors: usize,
    /// The walk stopped at the file cap or its time budget: files past it were not read.
    pub truncated: bool,
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
    /// The walk of the folder stops after this long.
    pub walk_budget: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_files: MAX_FILES,
            max_file_bytes: MAX_FILE_BYTES,
            walk_budget: WALK_BUDGET,
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
    /// The walk stopped at the file cap or its time budget.
    truncated: bool,
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
        // The same order each time, so a capped walk reads the same files.
        .sort_by_file_name(|a, b| a.cmp(b))
        .filter_entry(|e| {
            let is_dir = e.file_type().is_some_and(|t| t.is_dir());
            !(is_dir && SKIP_DIRS.iter().any(|d| e.file_name() == *d))
        })
        .build();
    let mut files = Vec::new();
    let mut markers = Markers::default();
    let mut skipped = 0;
    let mut truncated = false;
    let started = Instant::now();
    for entry in walker.flatten() {
        // The caps hold during the walk, not after it: a folder of millions stops here.
        if files.len() >= limits.max_files || started.elapsed() >= limits.walk_budget {
            truncated = true;
            break;
        }
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
    Scan {
        files,
        markers,
        skipped,
        truncated,
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

    // Which files may have changed: their size or time differs from the stored row. Workers
    // then read, hash and (when the hash differs) parse each one; a file's bytes live only in
    // its worker, so memory holds one file per thread.
    let total_check = scan.files.len();
    let to_check: Vec<usize> = (0..scan.files.len())
        .filter(|i| {
            let c = &scan.files[*i];
            !rows
                .get(&c.path)
                .is_some_and(|r| r.size == c.size && r.mtime == c.mtime)
        })
        .collect();
    let total = total_check + to_check.len();
    progress(total_check, total);
    enum Outcome {
        Unreadable(usize),
        Touched(Changed),
        Parsed(Changed),
        Panicked(usize),
    }
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let outcomes: Mutex<Vec<Outcome>> = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 4)
        .min(to_check.len().max(1));
    std::thread::scope(|s| {
        for _ in 0..threads {
            let (files, to_check, next, done, outcomes, rows) =
                (&scan.files, &to_check, &next, &done, &outcomes, &rows);
            let worker = move || loop {
                let k = next.fetch_add(1, Ordering::Relaxed);
                let Some(&i) = to_check.get(k) else {
                    break;
                };
                let c = &files[i];
                let outcome = match read_text(&root.join(&c.path), limits.max_file_bytes) {
                    None => Outcome::Unreadable(i),
                    Some(bytes) => {
                        let h = hash(&bytes);
                        let row = FileRow {
                            size: c.size,
                            mtime: c.mtime,
                            hash: h,
                        };
                        if rows.get(&c.path).is_some_and(|r| r.hash == row.hash) {
                            Outcome::Touched(Changed {
                                path: c.path.clone(),
                                row,
                                lang: c.lang,
                                facts: None,
                            })
                        } else {
                            let lang = c.lang;
                            let parsed = std::panic::catch_unwind(move || {
                                let text = String::from_utf8_lossy(&bytes);
                                #[cfg(test)]
                                if text.starts_with(PANIC_MARKER) {
                                    panic!("a test asked the parser to panic");
                                }
                                extract(lang, &text).unwrap_or_default()
                            });
                            match parsed {
                                Ok(facts) => Outcome::Parsed(Changed {
                                    path: c.path.clone(),
                                    row,
                                    lang: c.lang,
                                    facts: Some(facts),
                                }),
                                Err(_) => Outcome::Panicked(i),
                            }
                        }
                    }
                };
                outcomes.lock().unwrap().push(outcome);
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n.is_multiple_of(50) {
                    progress(total_check + n, total);
                }
            };
            std::thread::Builder::new()
                .name("graph-parse".into())
                .stack_size(PARSE_STACK)
                .spawn_scoped(s, worker)
                .expect("a parse thread starts");
        }
    });
    let mut parsed: Vec<Changed> = Vec::new();
    let mut touched: Vec<Changed> = Vec::new();
    let mut unreadable: HashSet<&str> = HashSet::new();
    let mut parse_errors = 0;
    for outcome in outcomes.into_inner().unwrap() {
        match outcome {
            Outcome::Unreadable(i) => {
                skipped += 1;
                unreadable.insert(scan.files[i].path.as_str());
            }
            Outcome::Panicked(i) => {
                skipped += 1;
                parse_errors += 1;
                unreadable.insert(scan.files[i].path.as_str());
            }
            Outcome::Touched(c) => touched.push(c),
            Outcome::Parsed(c) => parsed.push(c),
        }
    }
    parsed.sort_by(|a, b| a.path.cmp(&b.path));

    let present: HashSet<&str> = scan
        .files
        .iter()
        .map(|c| c.path.as_str())
        .filter(|p| !unreadable.contains(p))
        .collect();
    // Nothing changed since a good build: the graph stands as it is.
    let unchanged = parsed.is_empty()
        && !rows.keys().any(|p| !present.contains(p.as_str()))
        && store::read_markers(&conn).as_ref() == Some(&scan.markers)
        && parse_errors == 0
        && !scan.truncated;
    if unchanged {
        if let Some(mut meta) = store::read_meta(&conn).filter(|m| {
            m.built_at.is_some() && m.error.is_none() && m.parse_errors == 0 && !m.truncated
        }) {
            store::touch_files(&mut conn, &touched)
                .map_err(|e| format!("writing the graph failed: {e}"))?;
            // Checked and found up to date: the graph says so with the time.
            meta.built_at = Some(crate::util::now());
            meta.duration_ms = started.elapsed().as_millis() as u64;
            store::write_meta(&conn, &meta)
                .map_err(|e| format!("writing the graph failed: {e}"))?;
            progress(total, total);
            return Ok(Report {
                parse_errors: 0,
                truncated: false,
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
        parse_errors: parse_errors as u64,
        truncated: scan.truncated,
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
        parse_errors,
        truncated: scan.truncated,
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
            walk_budget: WALK_BUDGET,
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

        // Nothing changed: nothing is parsed, and the graph says when it was last checked.
        let built_at = |db: &Path| {
            let conn = store::open(db).unwrap();
            store::read_meta(&conn).unwrap().built_at.unwrap()
        };
        let before = built_at(&db);
        std::thread::sleep(std::time::Duration::from_millis(20));
        let again = build(&root, &db, None, limits, &quiet).unwrap();
        assert_eq!(again.parsed, 0);
        assert_eq!(again.nodes, first.nodes);
        assert!(built_at(&db) > before);

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
    fn the_walk_stops_at_the_file_cap_and_says_so() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        for i in 0..5 {
            write(&root, &format!("f{i}.py"), "def f(): pass\n");
        }
        let limits = Limits {
            max_files: 3,
            max_file_bytes: 1000,
            walk_budget: WALK_BUDGET,
        };
        let db = tmp.path().join("g.sqlite");
        let r = build(&root, &db, None, limits, &quiet).unwrap();
        assert_eq!(r.files, 3);
        assert!(r.truncated);
        // The same three each time: the walk's order is fixed.
        let first = ids(&db);
        build(&root, &db, None, limits, &quiet).unwrap();
        assert_eq!(ids(&db), first);
        let conn = store::open(&db).unwrap();
        assert!(store::read_meta(&conn).unwrap().truncated);
    }

    #[test]
    fn the_walk_stops_at_its_time_budget() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        write(&root, "a.py", "def f(): pass\n");
        let limits = Limits {
            walk_budget: Duration::ZERO,
            ..Limits::default()
        };
        let r = build(&root, &tmp.path().join("g.sqlite"), None, limits, &quiet).unwrap();
        assert!(r.truncated);
        assert_eq!(r.files, 0);
    }

    #[test]
    fn a_parse_that_panics_is_skipped_and_counted() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        write(&root, "good.py", "def f(): pass\n");
        write(&root, "bad.py", &format!("{PANIC_MARKER}\ndef g(): pass\n"));
        let db = tmp.path().join("g.sqlite");
        let r = build(&root, &db, None, Limits::default(), &quiet).unwrap();
        assert_eq!((r.files, r.parse_errors, r.skipped), (1, 1, 1));
        let conn = store::open(&db).unwrap();
        assert_eq!(store::read_meta(&conn).unwrap().parse_errors, 1);
        assert!(ids(&db).contains(&"good.py#f".to_string()));
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
