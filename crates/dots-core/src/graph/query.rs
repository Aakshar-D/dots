//! The `graph` tool's actions (project graph spec section 8) over a loaded graph. Read-only;
//! every answer is compact and capped.

use std::collections::{BTreeMap, HashMap, VecDeque};

use serde_json::{json, Value};

use super::model::{EdgeKind, Graph, Node, NodeKind};
use super::store::{Maps, Meta};

/// A tool answer is capped at this many bytes.
pub const MAX_OUTPUT: usize = 12 * 1024;
const MAX_SEARCH: usize = 30;
const PER_GROUP: usize = 15;
const MAX_NEIGHBORS: usize = 60;
const MAX_PATH: usize = 8;
const MAX_LIST: usize = 200;

/// What the tool tells the model it does.
pub const DESCRIPTION: &str = "Ask the code graph Sage keeps of this project instead of \
    reading or grepping widely. Actions: map (the project's layout, entry points, clusters, \
    hubs), search (query: a name or path), node (node: one symbol or file with its links), \
    neighbors (node, depth 1 or 2), path (node to `to`), callers and callees (node), file \
    (path: the symbols of a file). A node is an id from an answer, or a name or path.";

/// The tool's arguments, as JSON Schema properties; only `action` is required.
pub fn properties() -> Value {
    let s = |d: &str| json!({"type": "string", "description": d});
    json!({
        "action": {
            "type": "string",
            "enum": ["map", "search", "node", "neighbors", "path", "callers", "callees", "file"],
            "description": "What to ask"
        },
        "query": s("search: words of a name or path"),
        "node": s("node, neighbors, path, callers, callees: a node id, name or path"),
        "to": s("path: the node to reach"),
        "path": s("file: a file path relative to the project root"),
        "kind": s("search: only nodes of this kind (file, type, function, method, heading, package, dir)"),
        "depth": {"type": "integer", "description": "neighbors: 1 or 2"}
    })
}

/// A graph loaded for answering, with its edges indexed both ways.
pub struct Loaded {
    pub graph: Graph,
    pub maps: Maps,
    pub meta: Meta,
    index: HashMap<String, u32>,
    out: Vec<Vec<u32>>,
    inc: Vec<Vec<u32>>,
}

impl Loaded {
    pub fn new(graph: Graph, maps: Maps, meta: Meta) -> Loaded {
        let n = graph.nodes.len();
        let mut out = vec![Vec::new(); n];
        let mut inc = vec![Vec::new(); n];
        for (i, e) in graph.edges.iter().enumerate() {
            out[e.from as usize].push(i as u32);
            inc[e.to as usize].push(i as u32);
        }
        let index = graph
            .nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.clone(), i as u32))
            .collect();
        Loaded {
            graph,
            maps,
            meta,
            index,
            out,
            inc,
        }
    }

    pub fn node(&self, i: u32) -> &Node {
        &self.graph.nodes[i as usize]
    }

    pub fn find_id(&self, id: &str) -> Option<u32> {
        self.index.get(id).copied()
    }

    /// The node `reference` names: an id, a file path, or the best match of a search.
    pub fn resolve(&self, reference: &str) -> Result<u32, String> {
        let r = reference.trim();
        if r.is_empty() {
            return Err("node is required".into());
        }
        for id in [
            r.to_string(),
            format!("file:{r}"),
            format!("dir:{r}"),
            format!("pkg:{r}"),
        ] {
            if let Some(i) = self.find_id(&id) {
                return Ok(i);
            }
        }
        let hits = self.search(r, None);
        match hits.first() {
            Some((i, rank)) if *rank <= 1 => Ok(*i),
            Some(_) => Err(format!(
                "no node is named {r:?}. Close matches:\n{}",
                hits.iter()
                    .take(8)
                    .map(|(i, _)| format!("- {}", self.line(*i)))
                    .collect::<Vec<_>>()
                    .join("\n")
            )),
            None => Err(format!(
                "no node matches {r:?}. Try action search with part of the name."
            )),
        }
    }

    /// The nodes matching `query`, best first, with their rank: 0 the exact name, 1 a name
    /// that starts with it, 2 one that has it, 3 an id or path that has it.
    pub fn search(&self, query: &str, kind: Option<NodeKind>) -> Vec<(u32, u8)> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let terms: Vec<&str> = q.split_whitespace().collect();
        let mut hits: Vec<(u32, u8)> = Vec::new();
        for (i, n) in self.graph.nodes.iter().enumerate() {
            if kind.is_some_and(|k| k != n.kind) {
                continue;
            }
            let name = n.name.to_lowercase();
            let last = name.rsplit('.').next().unwrap_or(&name);
            let id = n.id.to_lowercase();
            let rank = if name == q || last == q {
                0
            } else if name.starts_with(&q) || last.starts_with(&q) {
                1
            } else if name.contains(&q) {
                2
            } else if terms.iter().all(|t| id.contains(t)) {
                3
            } else {
                continue;
            };
            hits.push((i as u32, rank));
        }
        hits.sort_by(|a, b| {
            a.1.cmp(&b.1)
                .then(kind_rank(self.node(a.0).kind).cmp(&kind_rank(self.node(b.0).kind)))
                .then(self.node(b.0).degree.cmp(&self.node(a.0).degree))
                .then(self.node(a.0).id.cmp(&self.node(b.0).id))
        });
        hits
    }

    /// A node on one line: `id :line (kind)`.
    pub fn line(&self, i: u32) -> String {
        let n = self.node(i);
        match n.kind {
            NodeKind::Dir | NodeKind::Package | NodeKind::File => {
                format!("{} ({})", n.id, n.kind.as_str())
            }
            _ => format!("{} :{} ({})", n.id, n.line, n.kind.as_str()),
        }
    }
}

/// Symbols before files, files before folders and packages.
fn kind_rank(k: NodeKind) -> u8 {
    match k {
        NodeKind::Type | NodeKind::Function | NodeKind::Method => 0,
        NodeKind::File => 1,
        NodeKind::Heading => 2,
        NodeKind::Dir | NodeKind::Package => 3,
    }
}

/// Text that stops growing at `MAX_OUTPUT`, with a note of what it left out.
struct Out {
    text: String,
    dropped: usize,
}

impl Out {
    fn new() -> Out {
        Out {
            text: String::new(),
            dropped: 0,
        }
    }

    fn push(&mut self, line: impl AsRef<str>) {
        let line = line.as_ref();
        if self.dropped > 0 || self.text.len() + line.len() + 1 > MAX_OUTPUT - 80 {
            self.dropped += 1;
            return;
        }
        self.text.push_str(line);
        self.text.push('\n');
    }

    fn done(mut self) -> String {
        if self.dropped > 0 {
            self.text.push_str(&format!(
                "[cut: {} more lines; ask a narrower question]",
                self.dropped
            ));
        }
        self.text.trim_end().to_string()
    }
}

/// Runs a `graph` call against `loaded`.
pub fn call(loaded: &Loaded, input: &Value) -> Result<String, String> {
    let text = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
    };
    let action = text("action").ok_or("action is required")?;
    let node = || {
        text("node")
            .ok_or_else(|| format!("{action} needs node"))
            .and_then(|r| loaded.resolve(r))
    };
    match action {
        "map" => Ok(loaded.maps.full.trim_end().to_string()),
        "search" => {
            let query = text("query").ok_or("search needs query")?;
            let kind = match text("kind") {
                Some(k) => Some(NodeKind::parse(k).ok_or_else(|| format!("unknown kind {k:?}"))?),
                None => None,
            };
            Ok(search(loaded, query, kind))
        }
        "node" => Ok(describe(loaded, node()?)),
        "neighbors" => {
            let depth = match input.get("depth") {
                None | Some(Value::Null) => 1,
                Some(v) => match v.as_u64() {
                    Some(d @ 1..=2) => d as usize,
                    _ => return Err("depth must be 1 or 2".into()),
                },
            };
            Ok(neighbors(loaded, node()?, depth))
        }
        "path" => {
            let from = node()?;
            let to = loaded.resolve(text("to").ok_or("path needs to")?)?;
            Ok(path(loaded, from, to))
        }
        "callers" => Ok(calls(loaded, node()?, false)),
        "callees" => Ok(calls(loaded, node()?, true)),
        "file" => {
            let p = text("path")
                .or_else(|| text("node"))
                .ok_or("file needs path")?;
            let p = p.trim_start_matches("file:").trim_start_matches("./");
            let p = p.replace('\\', "/");
            Ok(file(loaded, &p)?)
        }
        other => Err(format!(
            "unknown action {other:?}: use map, search, node, neighbors, path, callers, callees or file"
        )),
    }
}

fn search(loaded: &Loaded, query: &str, kind: Option<NodeKind>) -> String {
    let hits = loaded.search(query, kind);
    if hits.is_empty() {
        return format!("nothing matches {query:?}");
    }
    let mut out = Out::new();
    out.push(format!("{} matches for {query:?}:", hits.len()));
    for (i, _) in hits.iter().take(MAX_SEARCH) {
        out.push(format!("- {}", loaded.line(*i)));
    }
    if hits.len() > MAX_SEARCH {
        out.push(format!(
            "[{} more; narrow the query]",
            hits.len() - MAX_SEARCH
        ));
    }
    out.done()
}

fn describe(loaded: &Loaded, i: u32) -> String {
    let n = loaded.node(i);
    let mut out = Out::new();
    out.push(loaded.line(i));
    if let Some(file) = &n.file {
        if n.line > 0 {
            out.push(format!("at {file}:{}", n.line));
        }
    }
    if !n.signature.is_empty() {
        out.push(format!("signature: {}", n.signature));
    }
    if let Some(c) = loaded.graph.communities.get(n.community as usize) {
        out.push(format!("cluster {}: {}", c.id + 1, c.name));
    }
    // Neighbours by edge kind and direction.
    let mut groups: BTreeMap<(u8, EdgeKind), Vec<(u32, bool)>> = BTreeMap::new();
    for e in &loaded.out[i as usize] {
        let e = &loaded.graph.edges[*e as usize];
        groups.entry((0, e.kind)).or_default().push((e.to, e.exact));
    }
    for e in &loaded.inc[i as usize] {
        let e = &loaded.graph.edges[*e as usize];
        groups
            .entry((1, e.kind))
            .or_default()
            .push((e.from, e.exact));
    }
    for ((dir, kind), mut list) in groups {
        list.sort_by(|a, b| loaded.node(a.0).id.cmp(&loaded.node(b.0).id));
        let words = if dir == 0 {
            kind.as_str()
        } else {
            kind.reverse_words()
        };
        out.push(format!("{words} ({}):", list.len()));
        for (j, exact) in list.iter().take(PER_GROUP) {
            let mark = if *exact { "" } else { " [inferred]" };
            out.push(format!("  - {}{mark}", loaded.line(*j)));
        }
        if list.len() > PER_GROUP {
            out.push(format!("  - and {} more", list.len() - PER_GROUP));
        }
    }
    out.done()
}

fn neighbors(loaded: &Loaded, start: u32, depth: usize) -> String {
    let mut seen: HashMap<u32, usize> = HashMap::from([(start, 0)]);
    let mut queue = VecDeque::from([start]);
    let mut found: Vec<(usize, u32)> = Vec::new();
    while let Some(u) = queue.pop_front() {
        let d = seen[&u];
        if d >= depth {
            continue;
        }
        let mut next: Vec<u32> = loaded.out[u as usize]
            .iter()
            .map(|e| loaded.graph.edges[*e as usize].to)
            .chain(
                loaded.inc[u as usize]
                    .iter()
                    .map(|e| loaded.graph.edges[*e as usize].from),
            )
            .collect();
        next.sort_by(|a, b| loaded.node(*a).id.cmp(&loaded.node(*b).id));
        next.dedup();
        for v in next {
            if seen.contains_key(&v) {
                continue;
            }
            seen.insert(v, d + 1);
            found.push((d + 1, v));
            queue.push_back(v);
        }
    }
    let mut out = Out::new();
    out.push(format!(
        "{} nodes within {depth} of {}:",
        found.len(),
        loaded.node(start).id
    ));
    for (d, v) in found.iter().take(MAX_NEIGHBORS) {
        out.push(format!("- [{d}] {}", loaded.line(*v)));
    }
    if found.len() > MAX_NEIGHBORS {
        out.push(format!(
            "[{} more; use node or depth 1]",
            found.len() - MAX_NEIGHBORS
        ));
    }
    out.done()
}

fn path(loaded: &Loaded, from: u32, to: u32) -> String {
    // Breadth-first over edges both ways; each step remembers how it was reached.
    let mut prev: HashMap<u32, (u32, u32, bool)> = HashMap::new();
    let mut queue = VecDeque::from([(from, 0usize)]);
    let mut seen = std::collections::HashSet::from([from]);
    while let Some((u, d)) = queue.pop_front() {
        if u == to || d >= MAX_PATH {
            continue;
        }
        let steps = loaded.out[u as usize]
            .iter()
            .map(|e| (loaded.graph.edges[*e as usize].to, *e, true))
            .chain(
                loaded.inc[u as usize]
                    .iter()
                    .map(|e| (loaded.graph.edges[*e as usize].from, *e, false)),
            );
        for (v, e, forward) in steps {
            if seen.insert(v) {
                prev.insert(v, (u, e, forward));
                queue.push_back((v, d + 1));
            }
        }
    }
    if from != to && !prev.contains_key(&to) {
        return format!(
            "no path of at most {MAX_PATH} steps from {} to {}",
            loaded.node(from).id,
            loaded.node(to).id
        );
    }
    let mut chain = vec![to];
    let mut steps = Vec::new();
    let mut at = to;
    while at != from {
        let (p, e, forward) = prev[&at];
        steps.push((e, forward));
        chain.push(p);
        at = p;
    }
    chain.reverse();
    steps.reverse();
    let mut out = Out::new();
    out.push(format!("{} steps:", steps.len()));
    out.push(format!("- {}", loaded.line(chain[0])));
    for (k, (e, forward)) in steps.iter().enumerate() {
        let kind = loaded.graph.edges[*e as usize].kind;
        let words = if *forward {
            kind.as_str()
        } else {
            kind.reverse_words()
        };
        out.push(format!("  {words} -> {}", loaded.line(chain[k + 1])));
    }
    out.done()
}

fn calls(loaded: &Loaded, i: u32, outgoing: bool) -> String {
    let edges = if outgoing {
        &loaded.out[i as usize]
    } else {
        &loaded.inc[i as usize]
    };
    let mut list: Vec<(u32, bool, u32)> = edges
        .iter()
        .map(|e| &loaded.graph.edges[*e as usize])
        .filter(|e| e.kind == EdgeKind::Calls)
        .map(|e| (if outgoing { e.to } else { e.from }, e.exact, e.line))
        .collect();
    list.sort_by(|a, b| {
        loaded
            .node(a.0)
            .id
            .cmp(&loaded.node(b.0).id)
            .then(a.2.cmp(&b.2))
    });
    let n = loaded.node(i);
    let mut out = Out::new();
    let words = if outgoing { "calls" } else { "is called by" };
    if list.is_empty() {
        return format!(
            "{} {words} nothing the graph knows of (calls are matched by name)",
            n.id
        );
    }
    out.push(format!("{} {words} {}:", n.id, list.len()));
    for (j, exact, line) in list.iter().take(MAX_LIST) {
        let mark = if *exact { "" } else { " [inferred]" };
        out.push(format!("- {} (call at line {line}){mark}", loaded.line(*j)));
    }
    out.done()
}

fn file(loaded: &Loaded, path: &str) -> Result<String, String> {
    if loaded.find_id(&format!("file:{path}")).is_none() {
        let hits: Vec<String> = loaded
            .search(path, Some(NodeKind::File))
            .iter()
            .take(8)
            .map(|(i, _)| format!("- {}", loaded.line(*i)))
            .collect();
        return Err(if hits.is_empty() {
            format!("the graph has no file {path:?}")
        } else {
            format!(
                "the graph has no file {path:?}. Close matches:\n{}",
                hits.join("\n")
            )
        });
    }
    let mut symbols: Vec<&Node> = loaded
        .graph
        .nodes
        .iter()
        .filter(|n| n.file.as_deref() == Some(path) && n.kind != NodeKind::File)
        .collect();
    symbols.sort_by(|a, b| a.line.cmp(&b.line).then(a.id.cmp(&b.id)));
    let mut out = Out::new();
    out.push(format!("{path}: {} symbols", symbols.len()));
    for n in symbols.iter().take(MAX_LIST) {
        let sig = if n.signature.is_empty() {
            n.name.clone()
        } else {
            n.signature.clone()
        };
        out.push(format!(":{} {} {sig}", n.line, n.kind.as_str()));
    }
    if symbols.len() > MAX_LIST {
        out.push(format!("[{} more]", symbols.len() - MAX_LIST));
    }
    Ok(out.done())
}

#[cfg(test)]
mod tests {
    use super::super::extract::{extract, Lang};
    use super::super::link::{link, Markers, Parsed};
    use super::*;

    fn fixture() -> Loaded {
        let files = [
            (
                "src/runner.rs",
                "use crate::store::Store;\npub struct Runner;\nimpl Runner {\n    pub fn start(&self) { Store::open(); self.stop(); }\n    fn stop(&self) {}\n}\n",
            ),
            (
                "src/store.rs",
                "pub struct Store;\nimpl Store { pub fn open() {} }\n",
            ),
            ("src/lib.rs", "mod runner;\nmod store;\n"),
        ];
        let parsed = files
            .iter()
            .map(|(p, s)| {
                (
                    p.to_string(),
                    Parsed {
                        lang: Lang::Rust,
                        facts: extract(Lang::Rust, s).unwrap(),
                    },
                )
            })
            .collect();
        let mut markers = Markers::default();
        markers.cargo.insert(String::new(), "app".into());
        let mut g = link(&parsed, &markers);
        crate::graph::community::assign(&mut g);
        Loaded::new(
            g,
            Maps {
                prompt: "map".into(),
                full: "full map".into(),
            },
            Meta::default(),
        )
    }

    fn ask(l: &Loaded, v: Value) -> Result<String, String> {
        call(l, &v)
    }

    #[test]
    fn each_action_answers() {
        let l = fixture();
        assert_eq!(ask(&l, json!({"action": "map"})).unwrap(), "full map");
        let s = ask(&l, json!({"action": "search", "query": "start"})).unwrap();
        assert!(s.contains("src/runner.rs#Runner.start :4 (method)"), "{s}");
        let n = ask(&l, json!({"action": "node", "node": "Runner.start"})).unwrap();
        assert!(n.contains("calls (2):"), "{n}");
        assert!(n.contains("src/store.rs#Store.open"), "{n}");
        assert!(n.contains("contained in (1):"), "{n}");
        let c = ask(&l, json!({"action": "callers", "node": "Store.open"})).unwrap();
        assert!(c.contains("src/runner.rs#Runner.start"), "{c}");
        let c = ask(
            &l,
            json!({"action": "callees", "node": "src/runner.rs#Runner.start"}),
        )
        .unwrap();
        assert!(c.contains("Runner.stop") && c.contains("Store.open"), "{c}");
        let f = ask(&l, json!({"action": "file", "path": "src/runner.rs"})).unwrap();
        assert!(f.starts_with("src/runner.rs: 3 symbols"), "{f}");
        assert!(f.contains(":4 method pub fn start(&self)"), "{f}");
        let p = ask(
            &l,
            json!({"action": "path", "node": "Runner.stop", "to": "Store.open"}),
        )
        .unwrap();
        assert!(p.starts_with("2 steps:"), "{p}");
        let nb = ask(
            &l,
            json!({"action": "neighbors", "node": "src/store.rs", "depth": 2}),
        )
        .unwrap();
        assert!(nb.contains("[1] src/store.rs#Store"), "{nb}");
    }

    #[test]
    fn bad_input_is_explained() {
        let l = fixture();
        assert!(ask(&l, json!({}))
            .unwrap_err()
            .contains("action is required"));
        assert!(ask(&l, json!({"action": "fly"}))
            .unwrap_err()
            .contains("unknown action"));
        assert!(ask(&l, json!({"action": "node"}))
            .unwrap_err()
            .contains("needs node"));
        assert!(ask(&l, json!({"action": "node", "node": "zzz"}))
            .unwrap_err()
            .contains("no node matches"));
        assert!(ask(
            &l,
            json!({"action": "neighbors", "node": "Runner", "depth": 5})
        )
        .unwrap_err()
        .contains("depth"));
        assert!(ask(&l, json!({"action": "file", "path": "src/nope.rs"}))
            .unwrap_err()
            .contains("no file"));
    }

    #[test]
    fn answers_are_capped() {
        // A file with many functions: its listing stops under the cap with a note.
        let src: String = (0..3000)
            .map(|i| {
                format!("pub fn function_with_a_long_name_{i}(a: u32, b: u32) -> u32 {{ a }}\n")
            })
            .collect();
        let parsed = [(
            "src/big.rs".to_string(),
            Parsed {
                lang: Lang::Rust,
                facts: extract(Lang::Rust, &src).unwrap(),
            },
        )]
        .into_iter()
        .collect();
        let g = link(&parsed, &Markers::default());
        let l = Loaded::new(g, Maps::default(), Meta::default());
        let f = ask(&l, json!({"action": "file", "path": "src/big.rs"})).unwrap();
        assert!(f.len() <= MAX_OUTPUT, "{}", f.len());
        let s = ask(&l, json!({"action": "search", "query": "function"})).unwrap();
        assert!(s.len() <= MAX_OUTPUT);
        assert!(s.contains("more; narrow the query"), "{s}");
        let n = ask(&l, json!({"action": "node", "node": "src/big.rs"})).unwrap();
        assert!(n.contains("and 2985 more"), "{n}");
    }
}
