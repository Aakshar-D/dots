//! The graph file of a project (project graph spec section 6): one SQLite file under the data
//! folder, never in the repository. It keeps each file's facts, so a rebuild parses only what
//! changed, and the linked graph, so the tool and the screen read it without building.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::extract::{Facts, Lang};
use super::link::{Markers, Parsed};
use super::model::{Community, Edge, EdgeKind, Graph, Node, NodeKind};

/// The builder's version: a file of another version is rebuilt from scratch.
pub const VERSION: &str = "2";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS files (
    path TEXT PRIMARY KEY, size INTEGER NOT NULL, mtime INTEGER NOT NULL,
    hash TEXT NOT NULL, lang TEXT NOT NULL, facts TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS nodes (
    idx INTEGER PRIMARY KEY, id TEXT NOT NULL, kind TEXT NOT NULL, name TEXT NOT NULL,
    file TEXT, line INTEGER NOT NULL, signature TEXT NOT NULL, community INTEGER NOT NULL,
    degree INTEGER NOT NULL, command INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS edges (
    src INTEGER NOT NULL, dst INTEGER NOT NULL, kind TEXT NOT NULL,
    exact INTEGER NOT NULL, line INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS communities (
    id INTEGER PRIMARY KEY, name TEXT NOT NULL, nodes INTEGER NOT NULL, files INTEGER NOT NULL);
";

/// What a build recorded about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub root: String,
    pub built_at: Option<String>,
    pub duration_ms: u64,
    pub files: u64,
    pub skipped: u64,
    pub nodes: u64,
    pub edges: u64,
    /// Language → files.
    pub langs: BTreeMap<String, usize>,
    pub commit: Option<String>,
    /// The last build's error; `None` when it succeeded.
    pub error: Option<String>,
    /// Files whose parse failed (counted in `skipped` too).
    #[serde(default)]
    pub parse_errors: u64,
    /// The walk stopped at the file cap or its time budget.
    #[serde(default)]
    pub truncated: bool,
}

/// A file as the last build saw it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRow {
    pub size: u64,
    pub mtime: i64,
    pub hash: String,
}

/// A file a build parsed (or found unchanged by its hash, `facts` then `None`).
pub struct Changed {
    pub path: String,
    pub row: FileRow,
    pub lang: Lang,
    pub facts: Option<Facts>,
}

/// Opens (making it when missing) the graph file at `path`.
pub fn open(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.execute_batch(SCHEMA)?;
    let version: Option<String> = conn
        .query_row("SELECT value FROM meta WHERE key = 'version'", [], |r| {
            r.get(0)
        })
        .optional()?;
    if version.as_deref() != Some(VERSION) {
        conn.execute_batch(
            "DELETE FROM meta; DELETE FROM files; DELETE FROM nodes; DELETE FROM edges;
             DELETE FROM communities;",
        )?;
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('version', ?1)",
            [VERSION],
        )?;
    }
    Ok(conn)
}

/// Opens the graph file at `path` to read it; `None` when there is none.
pub fn open_existing(path: &Path) -> Option<Connection> {
    if !path.is_file() {
        return None;
    }
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()?;
    let _ = conn.busy_timeout(std::time::Duration::from_secs(10));
    Some(conn)
}

fn get(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()
}

fn put(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

/// The meta of the last build; `None` before the first, or for a file of another version.
pub fn read_meta(conn: &Connection) -> Option<Meta> {
    if get(conn, "version").ok().flatten().as_deref() != Some(VERSION) {
        return None;
    }
    let text = get(conn, "meta").ok().flatten()?;
    serde_json::from_str(&text).ok()
}

/// Replaces the meta of the last build.
pub fn write_meta(conn: &Connection, meta: &Meta) -> rusqlite::Result<()> {
    put(
        conn,
        "meta",
        &serde_json::to_string(meta).unwrap_or_default(),
    )
}

/// Records `error` as the last build's, keeping what the last good build wrote.
pub fn set_error(conn: &Connection, root: &str, error: &str) -> rusqlite::Result<()> {
    let mut meta = read_meta(conn).unwrap_or_else(|| Meta {
        root: root.to_string(),
        ..Meta::default()
    });
    meta.error = Some(error.to_string());
    put(
        conn,
        "meta",
        &serde_json::to_string(&meta).unwrap_or_default(),
    )
}

/// The stored rows of the files, by path.
pub fn file_rows(conn: &Connection) -> rusqlite::Result<HashMap<String, FileRow>> {
    let mut stmt = conn.prepare("SELECT path, size, mtime, hash FROM files")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            FileRow {
                size: r.get::<_, i64>(1)? as u64,
                mtime: r.get(2)?,
                hash: r.get(3)?,
            },
        ))
    })?;
    rows.collect()
}

/// The stored facts of every file but those in `except`.
pub fn stored_facts(
    conn: &Connection,
    except: &std::collections::HashSet<&str>,
) -> rusqlite::Result<BTreeMap<String, Parsed>> {
    let mut stmt = conn.prepare("SELECT path, lang, facts FROM files")?;
    let mut rows = stmt.query([])?;
    let mut out = BTreeMap::new();
    while let Some(r) = rows.next()? {
        let path: String = r.get(0)?;
        if except.contains(path.as_str()) {
            continue;
        }
        let lang: String = r.get(1)?;
        let facts: String = r.get(2)?;
        let lang: Option<Lang> = serde_json::from_str(&format!("\"{lang}\"")).ok();
        let (Some(lang), Ok(facts)) = (lang, serde_json::from_str::<Facts>(&facts)) else {
            continue;
        };
        out.insert(path, Parsed { lang, facts });
    }
    Ok(out)
}

/// The maps a build wrote: the prompt's and the tool's.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Maps {
    pub prompt: String,
    pub full: String,
}

/// Writes a build in one transaction: the changed and deleted files, the graph, the maps and
/// the meta.
pub fn write_build(
    conn: &mut Connection,
    changed: &[Changed],
    deleted: &[String],
    graph: &Graph,
    maps: &Maps,
    markers: &Markers,
    meta: &Meta,
) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    {
        let mut del = tx.prepare("DELETE FROM files WHERE path = ?1")?;
        for path in deleted {
            del.execute([path])?;
        }
        let mut upsert = tx.prepare(
            "INSERT INTO files (path, size, mtime, hash, lang, facts) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(path) DO UPDATE SET size = excluded.size, mtime = excluded.mtime,
                 hash = excluded.hash, lang = excluded.lang, facts = excluded.facts",
        )?;
        let mut touch = tx.prepare("UPDATE files SET size = ?2, mtime = ?3 WHERE path = ?1")?;
        for c in changed {
            match &c.facts {
                Some(facts) => {
                    upsert.execute(params![
                        c.path,
                        c.row.size as i64,
                        c.row.mtime,
                        c.row.hash,
                        c.lang.name(),
                        serde_json::to_string(facts).unwrap_or_else(|_| "{}".into()),
                    ])?;
                }
                None => {
                    touch.execute(params![c.path, c.row.size as i64, c.row.mtime])?;
                }
            }
        }
        tx.execute_batch("DELETE FROM nodes; DELETE FROM edges; DELETE FROM communities;")?;
        let mut ins = tx.prepare(
            "INSERT INTO nodes (idx, id, kind, name, file, line, signature, community, degree, command)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        for (i, n) in graph.nodes.iter().enumerate() {
            ins.execute(params![
                i as i64,
                n.id,
                n.kind.as_str(),
                n.name,
                n.file,
                n.line,
                n.signature,
                n.community,
                n.degree,
                n.command as i64,
            ])?;
        }
        let mut ins = tx.prepare(
            "INSERT INTO edges (src, dst, kind, exact, line) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for e in &graph.edges {
            ins.execute(params![
                e.from,
                e.to,
                e.kind.as_str(),
                e.exact as i64,
                e.line
            ])?;
        }
        let mut ins =
            tx.prepare("INSERT INTO communities (id, name, nodes, files) VALUES (?1, ?2, ?3, ?4)")?;
        for c in &graph.communities {
            ins.execute(params![c.id, c.name, c.nodes, c.files])?;
        }
    }
    put(&tx, "map_prompt", &maps.prompt)?;
    put(&tx, "map_full", &maps.full)?;
    put(
        &tx,
        "markers",
        &serde_json::to_string(markers).unwrap_or_default(),
    )?;
    put(
        &tx,
        "meta",
        &serde_json::to_string(meta).unwrap_or_default(),
    )?;
    tx.commit()
}

/// The stored graph, its maps and meta.
pub fn load(conn: &Connection) -> rusqlite::Result<Option<(Graph, Maps, Meta)>> {
    // One read transaction: a build that commits meanwhile is seen whole or not at all.
    let tx = conn.unchecked_transaction()?;
    let loaded = load_in(&tx)?;
    tx.finish()?;
    Ok(loaded)
}

/// The prompt's map of the last good build, read alone.
pub fn read_prompt_map(conn: &Connection) -> Option<String> {
    read_meta(conn)?.built_at?;
    get(conn, "map_prompt").ok().flatten()
}

fn load_in(conn: &Connection) -> rusqlite::Result<Option<(Graph, Maps, Meta)>> {
    let Some(meta) = read_meta(conn) else {
        return Ok(None);
    };
    if meta.built_at.is_none() {
        return Ok(None);
    }
    let mut graph = Graph::default();
    let mut stmt = conn.prepare(
        "SELECT id, kind, name, file, line, signature, community, degree, command
         FROM nodes ORDER BY idx",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let kind: String = r.get(1)?;
        graph.nodes.push(Node {
            id: r.get(0)?,
            kind: NodeKind::parse(&kind).unwrap_or(NodeKind::File),
            name: r.get(2)?,
            file: r.get(3)?,
            line: r.get(4)?,
            signature: r.get(5)?,
            community: r.get(6)?,
            degree: r.get(7)?,
            command: r.get::<_, i64>(8)? != 0,
        });
    }
    let mut stmt = conn.prepare("SELECT src, dst, kind, exact, line FROM edges")?;
    let mut rows = stmt.query([])?;
    let n = graph.nodes.len() as u32;
    while let Some(r) = rows.next()? {
        let kind: String = r.get(2)?;
        let (from, to): (u32, u32) = (r.get(0)?, r.get(1)?);
        if from >= n || to >= n {
            continue;
        }
        graph.edges.push(Edge {
            from,
            to,
            kind: EdgeKind::parse(&kind).unwrap_or(EdgeKind::Contains),
            exact: r.get::<_, i64>(3)? != 0,
            line: r.get(4)?,
        });
    }
    let mut stmt = conn.prepare("SELECT id, name, nodes, files FROM communities ORDER BY id")?;
    graph.communities = stmt
        .query_map([], |r| {
            Ok(Community {
                id: r.get(0)?,
                name: r.get(1)?,
                nodes: r.get(2)?,
                files: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let maps = Maps {
        prompt: get(conn, "map_prompt")?.unwrap_or_default(),
        full: get(conn, "map_full")?.unwrap_or_default(),
    };
    Ok(Some((graph, maps, meta)))
}

/// The markers the last build saw (`Cargo.toml`, `package.json`, `go.mod` folders).
pub fn read_markers(conn: &Connection) -> Option<Markers> {
    let text = get(conn, "markers").ok().flatten()?;
    serde_json::from_str(&text).ok()
}

/// Records the new size and time of files whose text did not change.
pub fn touch_files(conn: &mut Connection, files: &[Changed]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    {
        let mut touch = tx.prepare("UPDATE files SET size = ?2, mtime = ?3 WHERE path = ?1")?;
        for c in files {
            touch.execute(params![c.path, c.row.size as i64, c.row.mtime])?;
        }
    }
    tx.commit()
}

/// Copies the graph file `conn` reads to `target` (`VACUUM INTO`, consistent under WAL), and
/// records `root` as the copy's root, for a worktree seeded from its main checkout.
pub fn copy_to(conn: &Connection, target: &Path, root: &Path) -> rusqlite::Result<()> {
    if let Some(dir) = target.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    conn.execute("VACUUM INTO ?1", [target.to_string_lossy()])?;
    let copy = open(target)?;
    if let Some(mut meta) = read_meta(&copy) {
        meta.root = root.to_string_lossy().into_owned();
        put(
            &copy,
            "meta",
            &serde_json::to_string(&meta).unwrap_or_default(),
        )?;
    }
    Ok(())
}
