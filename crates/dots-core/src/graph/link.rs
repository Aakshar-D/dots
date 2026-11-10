//! Linking (project graph spec section 5, step 2): the nodes and edges of the whole project
//! from the stored facts of its files, with imports and calls resolved by name. It reads no
//! source.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use super::extract::{Facts, Lang, SymKind};
use super::model::{Edge, EdgeKind, Graph, Node, NodeKind};

/// What the scan saw besides the parsed files: the folders of `Cargo.toml` files (with the
/// crate's name), of `package.json` files, and of `go.mod` files (with the module path).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Markers {
    pub cargo: BTreeMap<String, String>,
    pub package_json: BTreeSet<String>,
    pub go_mod: BTreeMap<String, String>,
}

/// A parsed file: its language and facts.
#[derive(Debug, Clone)]
pub struct Parsed {
    pub lang: Lang,
    pub facts: Facts,
}

/// What an imported name stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    File(String),
    Dir(String),
    Item(String, String),
    Package(String),
}

struct Builder {
    nodes: Vec<Node>,
    index: HashMap<String, u32>,
    edges: Vec<Edge>,
    seen: HashMap<(u32, u32, EdgeKind), usize>,
}

impl Builder {
    fn node(
        &mut self,
        id: String,
        kind: NodeKind,
        name: &str,
        file: Option<&str>,
        line: u32,
        signature: &str,
    ) -> u32 {
        if let Some(i) = self.index.get(&id) {
            return *i;
        }
        let i = self.nodes.len() as u32;
        self.index.insert(id.clone(), i);
        self.nodes.push(Node {
            id,
            kind,
            name: name.to_string(),
            file: file.map(str::to_string),
            line,
            signature: signature.to_string(),
            community: 0,
            degree: 0,
            command: false,
        });
        i
    }

    fn edge(&mut self, from: u32, to: u32, kind: EdgeKind, exact: bool, line: u32) {
        if from == to {
            return;
        }
        match self.seen.get(&(from, to, kind)) {
            Some(at) => {
                if exact {
                    self.edges[*at].exact = true;
                }
            }
            None => {
                self.seen.insert((from, to, kind), self.edges.len());
                self.edges.push(Edge {
                    from,
                    to,
                    kind,
                    exact,
                    line,
                });
            }
        }
    }
}

fn parent_dir(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// `a/./b/../c` as `a/c`; `None` when it climbs above the root.
fn normalize(path: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop()?;
            }
            p => out.push(p),
        }
    }
    Some(out.join("/"))
}

fn join(dir: &str, rel: &str) -> String {
    if dir.is_empty() {
        rel.to_string()
    } else {
        format!("{dir}/{rel}")
    }
}

/// The nearest folder at or above `dir` in `set`.
fn nearest<I: Fn(&str) -> bool>(dir: &str, has: I) -> Option<&str> {
    let mut d = dir;
    loop {
        if has(d) {
            return Some(d);
        }
        if d.is_empty() {
            return None;
        }
        d = parent_dir(d);
    }
}

/// The graph of `files` (by path), without communities.
pub fn link(files: &BTreeMap<String, Parsed>, markers: &Markers) -> Graph {
    let mut b = Builder {
        nodes: Vec::new(),
        index: HashMap::new(),
        edges: Vec::new(),
        seen: HashMap::new(),
    };
    let paths: HashSet<&str> = files.keys().map(String::as_str).collect();

    // Folders and files.
    let mut file_node: HashMap<&str, u32> = HashMap::new();
    for path in files.keys() {
        let dir = parent_dir(path);
        let mut parent: Option<u32> = None;
        if !dir.is_empty() {
            let mut acc = String::new();
            for part in dir.split('/') {
                acc = join(&acc, part);
                let d = b.node(format!("dir:{acc}"), NodeKind::Dir, &acc, Some(&acc), 0, "");
                if let Some(p) = parent {
                    b.edge(p, d, EdgeKind::Contains, true, 0);
                }
                parent = Some(d);
            }
        }
        let name = path.rsplit('/').next().unwrap_or(path);
        let f = b.node(
            format!("file:{path}"),
            NodeKind::File,
            name,
            Some(path),
            0,
            "",
        );
        if let Some(p) = parent {
            b.edge(p, f, EdgeKind::Contains, true, 0);
        }
        file_node.insert(path.as_str(), f);
    }

    // Symbols.
    let mut sym_nodes: HashMap<&str, Vec<u32>> = HashMap::new();
    // (file, name) -> top-level symbols (not methods, not headings).
    let mut top: HashMap<(&str, &str), Vec<u32>> = HashMap::new();
    // (owner, method) -> methods.
    let mut methods: HashMap<(&str, &str), Vec<u32>> = HashMap::new();
    // name -> callable or type symbols, project-wide.
    let mut by_name: HashMap<&str, Vec<u32>> = HashMap::new();
    let mut types: HashMap<&str, Vec<u32>> = HashMap::new();
    for (path, parsed) in files {
        let f = file_node[path.as_str()];
        let mut ids = Vec::with_capacity(parsed.facts.symbols.len());
        for s in &parsed.facts.symbols {
            let qual = match (&s.owner, s.kind) {
                (Some(o), SymKind::Method) => format!("{o}.{}", s.name),
                _ => s.name.clone(),
            };
            let mut id = format!("{path}#{qual}");
            if b.index.contains_key(&id) {
                id = format!("{id}@{}", s.line);
            }
            let kind = match s.kind {
                SymKind::Type => NodeKind::Type,
                SymKind::Function => NodeKind::Function,
                SymKind::Method => NodeKind::Method,
                SymKind::Heading => NodeKind::Heading,
            };
            let n = b.node(id, kind, &qual, Some(path), s.line, &s.signature);
            b.nodes[n as usize].command = s.command;
            ids.push(n);
        }
        for (i, s) in parsed.facts.symbols.iter().enumerate() {
            let n = ids[i];
            let parent = match (s.kind, &s.owner, s.parent) {
                (SymKind::Heading, _, Some(p)) => ids.get(p as usize).copied(),
                (SymKind::Method, Some(owner), _) => parsed
                    .facts
                    .symbols
                    .iter()
                    .position(|t| t.kind == SymKind::Type && &t.name == owner)
                    .map(|t| ids[t]),
                _ => None,
            };
            b.edge(parent.unwrap_or(f), n, EdgeKind::Contains, true, s.line);
            match s.kind {
                SymKind::Heading => {}
                SymKind::Method => {
                    if let Some(owner) = &s.owner {
                        methods
                            .entry((owner.as_str(), s.name.as_str()))
                            .or_default()
                            .push(n);
                    }
                    by_name.entry(s.name.as_str()).or_default().push(n);
                }
                SymKind::Type | SymKind::Function => {
                    top.entry((path.as_str(), s.name.as_str()))
                        .or_default()
                        .push(n);
                    by_name.entry(s.name.as_str()).or_default().push(n);
                    if s.kind == SymKind::Type {
                        types.entry(s.name.as_str()).or_default().push(n);
                    }
                }
            }
        }
        sym_nodes.insert(path.as_str(), ids);
    }

    // Files by folder, for Go packages.
    let mut dir_files: HashMap<&str, Vec<&str>> = HashMap::new();
    for path in files.keys() {
        dir_files
            .entry(parent_dir(path))
            .or_default()
            .push(path.as_str());
    }
    let crate_names: HashMap<String, &str> = markers
        .cargo
        .iter()
        .map(|(dir, name)| (name.replace('-', "_"), dir.as_str()))
        .collect();

    // Imports.
    let mut bindings: HashMap<&str, HashMap<String, Target>> = HashMap::new();
    for (path, parsed) in files {
        let f = file_node[path.as_str()];
        let mut bound: HashMap<String, Target> = HashMap::new();
        for imp in &parsed.facts.imports {
            let resolved: Vec<(String, Target)> = match parsed.lang {
                Lang::Rust => rust_import(path, imp, &paths, markers, &crate_names),
                Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
                    script_import(path, imp, &paths, markers)
                }
                Lang::Python => python_import(path, imp, &paths),
                Lang::Go => go_import(imp, markers, &dir_files),
                Lang::Markdown => Vec::new(),
            };
            for (local, target) in resolved {
                let to = match &target {
                    Target::File(p) | Target::Item(p, _) => file_node.get(p.as_str()).copied(),
                    Target::Dir(d) => b.index.get(&format!("dir:{d}")).copied(),
                    Target::Package(name) => {
                        Some(b.node(format!("pkg:{name}"), NodeKind::Package, name, None, 0, ""))
                    }
                };
                if let Some(to) = to {
                    b.edge(f, to, EdgeKind::Imports, true, imp.line);
                }
                if !local.is_empty() && local != "*" {
                    bound.insert(local, target);
                }
            }
        }
        bindings.insert(path.as_str(), bound);
    }

    // Calls.
    let empty = HashMap::new();
    for (path, parsed) in files {
        let f = file_node[path.as_str()];
        let ids = &sym_nodes[path.as_str()];
        let bound = bindings.get(path.as_str()).unwrap_or(&empty);
        for call in &parsed.facts.calls {
            let caller = call.from.and_then(|i| ids.get(i as usize).copied());
            let owner = call
                .from
                .and_then(|i| parsed.facts.symbols.get(i as usize))
                .and_then(|s| s.owner.as_deref());
            let found = resolve_call(
                path,
                parsed.lang,
                call,
                owner,
                bound,
                &Index {
                    top: &top,
                    methods: &methods,
                    by_name: &by_name,
                    types: &types,
                    dir_files: &dir_files,
                    nodes: &b.nodes,
                },
            );
            if let Some((to, exact)) = found {
                b.edge(caller.unwrap_or(f), to, EdgeKind::Calls, exact, call.line);
            }
        }
    }

    // Bases.
    for (path, parsed) in files {
        let bound = bindings.get(path.as_str()).unwrap_or(&empty);
        for base in &parsed.facts.bases {
            let of = top
                .get(&(path.as_str(), base.of.as_str()))
                .and_then(|v| {
                    v.iter()
                        .copied()
                        .find(|n| b.nodes[*n as usize].kind == NodeKind::Type)
                })
                .or_else(|| unique(types.get(base.of.as_str())));
            let Some(of) = of else {
                continue;
            };
            let target = match bound.get(&base.name) {
                Some(Target::Item(p, item)) => top
                    .get(&(p.as_str(), item.as_str()))
                    .and_then(|v| v.first().copied())
                    .map(|n| (n, true)),
                _ => None,
            }
            .or_else(|| {
                top.get(&(path.as_str(), base.name.as_str()))
                    .and_then(|v| v.first().copied())
                    .map(|n| (n, true))
            })
            .or_else(|| unique(types.get(base.name.as_str())).map(|n| (n, false)));
            if let Some((to, exact)) = target {
                let kind = if base.implements {
                    EdgeKind::Implements
                } else {
                    EdgeKind::Extends
                };
                b.edge(of, to, kind, exact, base.line);
            }
        }
    }

    // Markdown mentions.
    for (path, parsed) in files {
        let ids = &sym_nodes[path.as_str()];
        for m in &parsed.facts.mentions {
            let Some(from) = ids.get(m.from as usize).copied() else {
                continue;
            };
            let target = if paths.contains(m.path.as_str()) {
                Some(m.path.clone())
            } else {
                let suffix = format!("/{}", m.path);
                let mut hits = files.keys().filter(|p| p.ends_with(&suffix));
                match (hits.next(), hits.next()) {
                    (Some(one), None) => Some(one.clone()),
                    _ => None,
                }
            };
            if let Some(t) = target {
                let exact = t == m.path;
                b.edge(from, file_node[t.as_str()], EdgeKind::Mentions, exact, 0);
            }
        }
    }

    let mut graph = Graph {
        nodes: b.nodes,
        edges: b.edges,
        communities: Vec::new(),
    };
    graph.count_degrees();
    graph
}

fn unique(v: Option<&Vec<u32>>) -> Option<u32> {
    match v {
        Some(v) if v.len() == 1 => Some(v[0]),
        _ => None,
    }
}

struct Index<'a> {
    top: &'a HashMap<(&'a str, &'a str), Vec<u32>>,
    methods: &'a HashMap<(&'a str, &'a str), Vec<u32>>,
    by_name: &'a HashMap<&'a str, Vec<u32>>,
    types: &'a HashMap<&'a str, Vec<u32>>,
    dir_files: &'a HashMap<&'a str, Vec<&'a str>>,
    nodes: &'a [Node],
}

impl Index<'_> {
    fn top_in(&self, file: &str, name: &str) -> Option<u32> {
        self.top.get(&(file, name)).and_then(|v| v.first().copied())
    }

    /// The method `name` of the type `owner`, the one in `file` first.
    fn method(&self, owner: &str, name: &str, file: Option<&str>) -> Option<u32> {
        let v = self.methods.get(&(owner, name))?;
        file.and_then(|f| {
            v.iter()
                .copied()
                .find(|n| self.nodes[*n as usize].file.as_deref() == Some(f))
        })
        .or_else(|| v.first().copied())
    }
}

/// Method names so common in the standard libraries (`x.map()`, `r.expect()`, `q.bind()`) that
/// a call on an unknown value is never matched to the project's one method of that name.
const COMMON_METHODS: &[&str] = &[
    "add",
    "all",
    "any",
    "append",
    "apply",
    "as_ref",
    "as_str",
    "bind",
    "build",
    "call",
    "catch",
    "clear",
    "clone",
    "close",
    "collect",
    "concat",
    "contains",
    "count",
    "default",
    "delete",
    "emit",
    "entry",
    "every",
    "execute",
    "expect",
    "extend",
    "fetch",
    "filter",
    "find",
    "finish",
    "first",
    "flush",
    "for_each",
    "forEach",
    "format",
    "from",
    "get",
    "handle",
    "has",
    "includes",
    "init",
    "insert",
    "into",
    "is_empty",
    "iter",
    "join",
    "keys",
    "last",
    "len",
    "list",
    "load",
    "lock",
    "map",
    "next",
    "on",
    "open",
    "parse",
    "pop",
    "push",
    "push_str",
    "read",
    "reduce",
    "remove",
    "replace",
    "resolve",
    "run",
    "save",
    "send",
    "set",
    "slice",
    "some",
    "sort",
    "split",
    "spawn",
    "start",
    "stop",
    "then",
    "to_string",
    "toString",
    "trim",
    "unwrap",
    "update",
    "values",
    "wait",
    "write",
];

/// A method name unlikely to be a library's too: two words (`create_run`, `fetchAll`) or a
/// long one. `x.path()`, `x.sleep()`, `x.focus()` are left unmatched.
fn distinctive(name: &str) -> bool {
    name.contains('_')
        || name.chars().skip(1).any(|c| c.is_ascii_uppercase())
        || name.chars().count() >= 10
}

/// The node a call goes to, and whether it is exact.
fn resolve_call(
    path: &str,
    lang: Lang,
    call: &super::extract::Call,
    owner: Option<&str>,
    bound: &HashMap<String, Target>,
    ix: &Index,
) -> Option<(u32, bool)> {
    let name = call.name.as_str();
    if call.on_self {
        if let Some(n) = owner.and_then(|o| ix.method(o, name, Some(path))) {
            return Some((n, true));
        }
    } else if let Some(q) = call.qualifier.as_deref() {
        let q = if q == "Self" { owner.unwrap_or(q) } else { q };
        match bound.get(q) {
            Some(Target::File(p)) => return ix.top_in(p, name).map(|n| (n, true)),
            Some(Target::Dir(d)) => {
                return ix.dir_files.get(d.as_str()).and_then(|fs| {
                    fs.iter()
                        .find_map(|f| ix.top_in(f, name))
                        .map(|n| (n, true))
                })
            }
            Some(Target::Item(p, item)) => {
                if let Some(n) = ix.method(item, name, Some(p)) {
                    return Some((n, true));
                }
                // A Rust module imported by name: `use crate::a::b; b::f()`.
                if let Some(n) = ix.top_in(p, name) {
                    return Some((n, false));
                }
            }
            Some(Target::Package(_)) => return None,
            None => {}
        }
        if ix.top_in(path, q).is_some() {
            if let Some(n) = ix.method(q, name, Some(path)) {
                return Some((n, true));
            }
        }
        if ix.types.contains_key(q) {
            let n = ix.method(q, name, None)?;
            let one_type = ix.types.get(q).is_some_and(|v| v.len() == 1);
            return Some((n, one_type));
        }
        // `q` is a value (`store.get_run()`): matched by name below, as a method.
    } else if !call.method {
        if let Some(n) = ix.top_in(path, name) {
            return Some((n, true));
        }
        match bound.get(name) {
            Some(Target::Item(p, item)) => {
                if let Some(n) = ix.top_in(p, item) {
                    return Some((n, true));
                }
            }
            // A name from an outside package is never the project's.
            Some(Target::Package(_)) => return None,
            _ => {}
        }
        if lang == Lang::Go {
            let dir = parent_dir(path);
            if let Some(n) = ix
                .dir_files
                .get(dir)
                .and_then(|fs| fs.iter().find_map(|f| ix.top_in(f, name)))
            {
                return Some((n, true));
            }
        }
    }
    // Matched by name across the project: only when one definition has it. A call on a value
    // can only be a method's, and not one of the everyday names.
    let on_value = call.method || call.qualifier.is_some() || call.on_self;
    if on_value && (COMMON_METHODS.contains(&name) || !distinctive(name)) {
        return None;
    }
    let v = ix.by_name.get(name)?;
    let candidates: Vec<u32> = v
        .iter()
        .copied()
        .filter(|n| {
            let kind = ix.nodes[*n as usize].kind;
            if on_value {
                kind == NodeKind::Method
            } else {
                kind.callable() || kind == NodeKind::Type
            }
        })
        .collect();
    (candidates.len() == 1).then(|| (candidates[0], false))
}

// ---- Import resolution ----

/// The `src` folder of the crate a Rust file is in, and the file's module path in it.
fn rust_module(path: &str, markers: &Markers) -> (String, Vec<String>) {
    let crate_dir = nearest(parent_dir(path), |d| markers.cargo.contains_key(d));
    let src = crate_dir.map(|d| join(d, "src"));
    let (base, rel) = match &src {
        Some(s) if path.starts_with(&format!("{s}/")) => (s.clone(), &path[s.len() + 1..]),
        _ => (
            parent_dir(path).to_string(),
            path.rsplit('/').next().unwrap_or(path),
        ),
    };
    let rel = rel.trim_end_matches(".rs");
    let mut segs: Vec<String> = rel.split('/').map(str::to_string).collect();
    match segs.last().map(String::as_str) {
        Some("lib" | "main") if segs.len() == 1 => segs.clear(),
        Some("mod") => {
            segs.pop();
        }
        _ => {}
    }
    if segs.first().is_some_and(|s| s == "bin") {
        segs.clear();
    }
    // A file outside `src` (a test, `build.rs`) is the root of its own modules.
    if src
        .as_ref()
        .is_none_or(|s| !path.starts_with(&format!("{s}/")))
    {
        segs.clear();
    }
    (base, segs)
}

/// The file of the module `segs` of the crate whose source folder is `src`.
fn rust_module_file(src: &str, segs: &[String], paths: &HashSet<&str>) -> Option<String> {
    if segs.is_empty() {
        return ["lib.rs", "main.rs"]
            .iter()
            .map(|f| join(src, f))
            .find(|p| paths.contains(p.as_str()));
    }
    let base = join(src, &segs.join("/"));
    [format!("{base}.rs"), format!("{base}/mod.rs")]
        .into_iter()
        .find(|p| paths.contains(p.as_str()))
}

fn rust_import(
    path: &str,
    imp: &super::extract::Import,
    paths: &HashSet<&str>,
    markers: &Markers,
    crates: &HashMap<String, &str>,
) -> Vec<(String, Target)> {
    let segs: Vec<String> = imp.spec.split("::").map(str::to_string).collect();
    let local = imp
        .items
        .first()
        .map(|i| i.local.clone())
        .unwrap_or_else(|| segs.last().cloned().unwrap_or_default());
    let (src, here) = rust_module(path, markers);
    let Some(first) = segs.first().map(String::as_str) else {
        return Vec::new();
    };
    let (src, base, rest): (String, Vec<String>, &[String]) = match first {
        "crate" => (src, Vec::new(), &segs[1..]),
        "self" => (src, here.clone(), &segs[1..]),
        "super" => {
            let mut base = here.clone();
            let mut i = 0;
            while segs.get(i).is_some_and(|s| s == "super") {
                base.pop();
                i += 1;
            }
            (src, base, &segs[i..])
        }
        "std" | "core" | "alloc" => return vec![(local, Target::Package(first.to_string()))],
        name => match crates.get(name) {
            Some(dir) => (join(dir, "src"), Vec::new(), &segs[1..]),
            None => {
                let mut child = here.clone();
                child.push(name.to_string());
                if rust_module_file(&src, &child, paths).is_some() {
                    (src, here.clone(), &segs[..])
                } else {
                    return vec![(local, Target::Package(name.to_string()))];
                }
            }
        },
    };
    let rest: Vec<String> = rest.iter().filter(|s| *s != "*").cloned().collect();
    // The longest prefix that is a module.
    for k in (0..=rest.len()).rev() {
        let mut m = base.clone();
        m.extend(rest[..k].iter().cloned());
        if let Some(file) = rust_module_file(&src, &m, paths) {
            let target = if k == rest.len() {
                Target::File(file)
            } else {
                Target::Item(file, rest[k].clone())
            };
            let local = if imp.module {
                rest.last().cloned().unwrap_or(local)
            } else {
                local
            };
            return vec![(local, target)];
        }
    }
    Vec::new()
}

const SCRIPT_EXTS: &[&str] = &["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"];

fn script_file(base: &str, paths: &HashSet<&str>) -> Option<String> {
    if paths.contains(base) {
        return Some(base.to_string());
    }
    // `./a.js` written for `./a.ts`.
    if let Some(stem) = base.strip_suffix(".js") {
        for ext in ["ts", "tsx"] {
            let p = format!("{stem}.{ext}");
            if paths.contains(p.as_str()) {
                return Some(p);
            }
        }
    }
    for ext in SCRIPT_EXTS {
        let p = format!("{base}.{ext}");
        if paths.contains(p.as_str()) {
            return Some(p);
        }
    }
    for ext in SCRIPT_EXTS {
        let p = join(base, &format!("index.{ext}"));
        if paths.contains(p.as_str()) {
            return Some(p);
        }
    }
    None
}

fn script_import(
    path: &str,
    imp: &super::extract::Import,
    paths: &HashSet<&str>,
    markers: &Markers,
) -> Vec<(String, Target)> {
    let spec = imp.spec.as_str();
    let file = if spec.starts_with('.') {
        normalize(&join(parent_dir(path), spec)).and_then(|p| script_file(&p, paths))
    } else if let Some(rest) = spec.strip_prefix("@/").or_else(|| spec.strip_prefix("~/")) {
        let pkg = nearest(parent_dir(path), |d| markers.package_json.contains(d)).unwrap_or("");
        script_file(&join(pkg, &format!("src/{rest}")), paths)
    } else {
        let mut parts = spec.split('/');
        let first = parts.next().unwrap_or_default();
        let name = if first.starts_with('@') {
            format!("{first}/{}", parts.next().unwrap_or_default())
        } else {
            first.to_string()
        };
        if name.is_empty() || name.starts_with("node:") {
            return Vec::new();
        }
        let mut out = vec![(String::new(), Target::Package(name.clone()))];
        out.extend(
            imp.items
                .iter()
                .map(|i| (i.local.clone(), Target::Package(name.clone()))),
        );
        return out;
    };
    let Some(file) = file else {
        return Vec::new();
    };
    let mut out = vec![(String::new(), Target::File(file.clone()))];
    for item in &imp.items {
        let target = match item.name.as_str() {
            "" => Target::File(file.clone()),
            // A default export is usually named as it is imported.
            "default" => Target::Item(file.clone(), item.local.clone()),
            name => Target::Item(file.clone(), name.to_string()),
        };
        out.push((item.local.clone(), target));
    }
    out
}

fn python_module(base: &str, paths: &HashSet<&str>) -> Option<String> {
    [
        format!("{base}.py"),
        join(base, "__init__.py"),
        format!("{base}.pyi"),
    ]
    .into_iter()
    .find(|p| paths.contains(p.as_str()))
}

fn python_import(
    path: &str,
    imp: &super::extract::Import,
    paths: &HashSet<&str>,
) -> Vec<(String, Target)> {
    let spec = imp.spec.as_str();
    let dots = spec.chars().take_while(|c| *c == '.').count();
    let dotted = spec[dots..].replace('.', "/");
    let module_base: Vec<String> = if dots > 0 {
        let mut dir = parent_dir(path).to_string();
        for _ in 1..dots {
            dir = parent_dir(&dir).to_string();
        }
        vec![if dotted.is_empty() {
            dir
        } else {
            join(&dir, &dotted)
        }]
    } else {
        let mut bases = vec![dotted.clone(), format!("src/{dotted}")];
        let mut d = parent_dir(path);
        while !d.is_empty() {
            bases.push(join(d, &dotted));
            d = parent_dir(d);
        }
        bases
    };
    for base in &module_base {
        // `from . import x` names modules of a package.
        let module = python_module(base, paths);
        if module.is_none() && imp.items.is_empty() {
            continue;
        }
        let mut out = Vec::new();
        if let Some(m) = &module {
            out.push((String::new(), Target::File(m.clone())));
        }
        let mut any = module.is_some();
        for item in &imp.items {
            if item.name.is_empty() {
                if let Some(m) = &module {
                    out.push((item.local.clone(), Target::File(m.clone())));
                }
                continue;
            }
            if let Some(sub) = python_module(&join(base, &item.name), paths) {
                any = true;
                out.push((String::new(), Target::File(sub.clone())));
                out.push((item.local.clone(), Target::File(sub)));
            } else if let Some(m) = &module {
                out.push((
                    item.local.clone(),
                    Target::Item(m.clone(), item.name.clone()),
                ));
            }
        }
        if any {
            return out;
        }
    }
    if dots > 0 {
        return Vec::new();
    }
    let name = spec.split('.').next().unwrap_or_default().to_string();
    if name.is_empty() {
        return Vec::new();
    }
    let mut out = vec![(String::new(), Target::Package(name.clone()))];
    out.extend(
        imp.items
            .iter()
            .map(|i| (i.local.clone(), Target::Package(name.clone()))),
    );
    out
}

fn go_import(
    imp: &super::extract::Import,
    markers: &Markers,
    dir_files: &HashMap<&str, Vec<&str>>,
) -> Vec<(String, Target)> {
    let local = imp
        .items
        .first()
        .map(|i| i.local.clone())
        .unwrap_or_default();
    let module = markers
        .go_mod
        .iter()
        .filter(|(_, m)| imp.spec == **m || imp.spec.starts_with(&format!("{m}/")))
        .max_by_key(|(_, m)| m.len());
    if let Some((dir, m)) = module {
        let rest = imp.spec[m.len()..].trim_start_matches('/');
        let target = join(dir, rest);
        if dir_files.contains_key(target.as_str()) {
            return vec![(local, Target::Dir(target))];
        }
        return Vec::new();
    }
    let name = imp.spec.clone();
    vec![(local, Target::Package(name))]
}

#[cfg(test)]
mod tests {
    use super::super::extract::extract;
    use super::*;

    fn parse(files: &[(&str, &str)]) -> BTreeMap<String, Parsed> {
        files
            .iter()
            .map(|(p, src)| {
                let lang = Lang::of(p).unwrap();
                (
                    p.to_string(),
                    Parsed {
                        lang,
                        facts: extract(lang, src).unwrap(),
                    },
                )
            })
            .collect()
    }

    fn edges(g: &Graph, kind: EdgeKind) -> Vec<String> {
        let mut out: Vec<String> = g
            .edges
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| {
                format!(
                    "{} -> {}{}",
                    g.nodes[e.from as usize].id,
                    g.nodes[e.to as usize].id,
                    if e.exact { "" } else { " ?" }
                )
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn rust_imports_and_calls_resolve_across_modules() {
        let files = parse(&[
            (
                "core/src/lib.rs",
                "mod store;\nmod runner;\npub use runner::Runner;\n",
            ),
            (
                "core/src/store.rs",
                "pub struct Store;\nimpl Store { pub fn open() -> Store { Store } }\npub fn helper() {}\n",
            ),
            (
                "core/src/runner.rs",
                "use crate::store::{self, Store};\nuse tokio::spawn;\npub struct Runner;\npub trait Go { fn go(&self); }\nimpl Go for Runner { fn go(&self) { Store::open(); store::helper(); self.stop(); unique_name(); } }\nimpl Runner { fn stop(&self) {} }\n",
            ),
            ("core/src/other.rs", "pub fn unique_name() {}\n"),
        ]);
        let mut markers = Markers::default();
        markers.cargo.insert("core".into(), "dots-core".into());
        let g = link(&files, &markers);
        assert_eq!(
            edges(&g, EdgeKind::Imports),
            vec![
                "file:core/src/lib.rs -> file:core/src/runner.rs",
                "file:core/src/lib.rs -> file:core/src/store.rs",
                "file:core/src/runner.rs -> file:core/src/store.rs",
                "file:core/src/runner.rs -> pkg:tokio",
            ]
        );
        assert_eq!(
            edges(&g, EdgeKind::Calls),
            vec![
                "core/src/runner.rs#Runner.go -> core/src/other.rs#unique_name ?",
                "core/src/runner.rs#Runner.go -> core/src/runner.rs#Runner.stop",
                "core/src/runner.rs#Runner.go -> core/src/store.rs#Store.open",
                "core/src/runner.rs#Runner.go -> core/src/store.rs#helper",
            ]
        );
        assert_eq!(
            edges(&g, EdgeKind::Implements),
            vec!["core/src/runner.rs#Runner -> core/src/runner.rs#Go"]
        );
        // Folders contain files, files their symbols, types their methods.
        let contains = edges(&g, EdgeKind::Contains);
        assert!(contains.contains(&"dir:core -> dir:core/src".to_string()));
        assert!(contains.contains(&"dir:core/src -> file:core/src/store.rs".to_string()));
        assert!(contains
            .contains(&"core/src/store.rs#Store -> core/src/store.rs#Store.open".to_string()));
    }

    #[test]
    fn script_imports_resolve_relative_alias_and_packages() {
        let files = parse(&[
            (
                "ui/src/app.tsx",
                "import { Panel } from '@/components/Panel';\nimport api from './api';\nimport { useQuery } from '@tanstack/react-query';\nexport function App() { useQuery(); api(); return <Panel />; }\n",
            ),
            ("ui/src/components/Panel.tsx", "export function Panel() { return null; }\n"),
            ("ui/src/api/index.ts", "export default function api() {}\n"),
        ]);
        let mut markers = Markers::default();
        markers.package_json.insert("ui".into());
        let g = link(&files, &markers);
        assert_eq!(
            edges(&g, EdgeKind::Imports),
            vec![
                "file:ui/src/app.tsx -> file:ui/src/api/index.ts",
                "file:ui/src/app.tsx -> file:ui/src/components/Panel.tsx",
                "file:ui/src/app.tsx -> pkg:@tanstack/react-query",
            ]
        );
        assert_eq!(
            edges(&g, EdgeKind::Calls),
            vec![
                "ui/src/app.tsx#App -> ui/src/api/index.ts#api",
                "ui/src/app.tsx#App -> ui/src/components/Panel.tsx#Panel",
            ]
        );
    }

    #[test]
    fn python_and_go_resolve_their_modules() {
        let files = parse(&[
            ("app/models.py", "class User:\n    def save(self): pass\n"),
            (
                "app/views.py",
                "from .models import User\nimport os\nclass Admin(User):\n    def go(self):\n        self.save()\n        User()\n",
            ),
            ("svc/store/db.go", "package store\nfunc Open() {}\n"),
            (
                "svc/main.go",
                "package main\nimport \"example.com/svc/store\"\nfunc main() { store.Open() }\n",
            ),
        ]);
        let mut markers = Markers::default();
        markers
            .go_mod
            .insert("svc".into(), "example.com/svc".into());
        let g = link(&files, &markers);
        assert_eq!(
            edges(&g, EdgeKind::Imports),
            vec![
                "file:app/views.py -> file:app/models.py",
                "file:app/views.py -> pkg:os",
                "file:svc/main.go -> dir:svc/store",
            ]
        );
        assert_eq!(
            edges(&g, EdgeKind::Extends),
            vec!["app/views.py#Admin -> app/models.py#User"]
        );
        let calls = edges(&g, EdgeKind::Calls);
        assert!(calls.contains(&"app/views.py#Admin.go -> app/models.py#User".to_string()));
        assert!(calls.contains(&"svc/main.go#main -> svc/store/db.go#Open".to_string()));
    }

    #[test]
    fn an_ambiguous_name_is_dropped_and_markdown_mentions_link() {
        let files = parse(&[
            ("a.ts", "export function run() {}\n"),
            ("b.ts", "export function run() {}\n"),
            ("c.ts", "export function go() { run(); }\n"),
            ("docs/x.md", "# Notes\n\nSee a.ts and src/lib.rs.\n"),
        ]);
        let g = link(&files, &Markers::default());
        assert!(edges(&g, EdgeKind::Calls).is_empty());
        // `a.ts` has no folder, so it is no path; nothing is mentioned.
        assert!(edges(&g, EdgeKind::Mentions).is_empty());
        let files = parse(&[
            ("src/lib.rs", "fn x() {}\n"),
            ("docs/x.md", "# Notes\n\nSee src/lib.rs.\n"),
        ]);
        let g = link(&files, &Markers::default());
        assert_eq!(
            edges(&g, EdgeKind::Mentions),
            vec!["docs/x.md#Notes -> file:src/lib.rs"]
        );
    }
}
