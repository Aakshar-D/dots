//! The project map (project graph spec section 7): the layout, entry points, clusters and
//! hubs of a graph, as short text. It holds no time, so it changes only when the graph does.

use std::collections::{BTreeMap, HashMap};

use super::model::{Graph, NodeKind};

/// How much a map shows.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub bytes: usize,
    pub layout: usize,
    pub entries: usize,
    pub clusters: usize,
    pub hubs: usize,
}

/// The map in the prompt: about 1.5k tokens.
pub const PROMPT: Limits = Limits {
    bytes: 6_000,
    layout: 12,
    entries: 12,
    clusters: 10,
    hubs: 15,
};

/// The map the tool's `map` gives.
pub const FULL: Limits = Limits {
    bytes: 12_000,
    layout: 25,
    entries: 25,
    clusters: 25,
    hubs: 30,
};

/// The last line of a map that was cut.
pub const CUT_NOTE: &str = "[map cut: call `graph` with action `map` for the longer one]";

fn top_dir(path: &str) -> &str {
    path.split_once('/').map_or("", |(d, _)| d)
}

/// The entry points: (file, what).
pub fn entry_points(graph: &Graph) -> Vec<(String, String)> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut commands: BTreeMap<String, usize> = BTreeMap::new();
    for n in &graph.nodes {
        let Some(file) = n.file.as_deref() else {
            continue;
        };
        match n.kind {
            NodeKind::File => {
                let name = n.name.as_str();
                let stem = name.split('.').next().unwrap_or(name);
                let what = match name {
                    "main.rs" | "lib.rs" | "main.go" | "__main__.py" | "manage.py" => {
                        Some("entry file")
                    }
                    _ if matches!(stem, "index" | "main")
                        && [".ts", ".tsx", ".js", ".jsx", ".mjs"]
                            .iter()
                            .any(|e| name.ends_with(e)) =>
                    {
                        Some("entry file")
                    }
                    _ if stem == "routes" || stem == "router" => Some("routes"),
                    _ => None,
                };
                if let Some(what) = what {
                    out.entry(file.to_string())
                        .or_default()
                        .push(what.to_string());
                }
            }
            NodeKind::Function if n.name == "main" => {
                out.entry(file.to_string())
                    .or_default()
                    .push("fn main".to_string());
            }
            NodeKind::Function | NodeKind::Method if n.command => {
                *commands.entry(file.to_string()).or_insert(0) += 1;
            }
            _ => {}
        }
    }
    for (file, count) in commands {
        let what = if count == 1 {
            "1 Tauri command".to_string()
        } else {
            format!("{count} Tauri commands")
        };
        out.entry(file).or_default().push(what);
    }
    let mut list: Vec<(String, String)> = out
        .into_iter()
        .map(|(f, mut w)| {
            w.dedup();
            (f, w.join(", "))
        })
        .collect();
    // Shallow files first: they are the more likely entries.
    list.sort_by(|a, b| {
        a.0.matches('/')
            .count()
            .cmp(&b.0.matches('/').count())
            .then(a.0.cmp(&b.0))
    });
    list
}

/// The map of `graph`, whose files are in `langs` (language → files), within `limits`.
pub fn render(graph: &Graph, langs: &BTreeMap<String, usize>, limits: Limits) -> String {
    let files: Vec<&str> = graph
        .nodes
        .iter()
        .filter(|n| n.kind == NodeKind::File)
        .filter_map(|n| n.file.as_deref())
        .collect();
    let links = graph
        .edges
        .iter()
        .filter(|e| e.kind != super::model::EdgeKind::Contains)
        .count();
    let mut lines: Vec<String> = Vec::new();
    lines.push("# Project map".to_string());
    let mut langs: Vec<(&String, &usize)> = langs.iter().collect();
    langs.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let langs: Vec<String> = langs.iter().map(|(l, n)| format!("{l} {n}")).collect();
    lines.push(format!(
        "{} files, {} nodes, {} links. Languages: {}.",
        files.len(),
        graph.nodes.len(),
        links,
        if langs.is_empty() {
            "none".to_string()
        } else {
            langs.join(", ")
        }
    ));

    // Layout: top folders, each with its largest subfolders.
    let mut tops: BTreeMap<&str, (usize, BTreeMap<&str, usize>)> = BTreeMap::new();
    for f in &files {
        let top = top_dir(f);
        let e = tops.entry(top).or_default();
        e.0 += 1;
        if !top.is_empty() {
            let rest = &f[top.len() + 1..];
            if let Some((sub, _)) = rest.split_once('/') {
                *e.1.entry(sub).or_insert(0) += 1;
            }
        }
    }
    type Top<'a> = (&'a str, (usize, BTreeMap<&'a str, usize>));
    let mut tops: Vec<Top> = tops.into_iter().collect();
    tops.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(b.0)));
    lines.push(String::new());
    lines.push("## Layout".to_string());
    for (top, (count, subs)) in tops.iter().take(limits.layout) {
        if top.is_empty() {
            lines.push(format!("- (root): {count} files"));
            continue;
        }
        let mut subs: Vec<(&&str, &usize)> = subs.iter().collect();
        subs.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let subs: Vec<String> = subs
            .iter()
            .take(6)
            .map(|(s, n)| format!("{s} {n}"))
            .collect();
        if subs.is_empty() {
            lines.push(format!("- {top}/: {count} files"));
        } else {
            lines.push(format!("- {top}/: {count} files ({})", subs.join(", ")));
        }
    }
    if tops.len() > limits.layout {
        lines.push(format!("- and {} more folders", tops.len() - limits.layout));
    }

    let entries = entry_points(graph);
    if !entries.is_empty() {
        lines.push(String::new());
        lines.push("## Entry points".to_string());
        for (file, what) in entries.iter().take(limits.entries) {
            lines.push(format!("- {file}: {what}"));
        }
        if entries.len() > limits.entries {
            lines.push(format!("- and {} more", entries.len() - limits.entries));
        }
    }

    // Each file's weight: the links of its nodes.
    let mut file_score: HashMap<&str, u32> = HashMap::new();
    for n in &graph.nodes {
        if let (Some(f), false) = (n.file.as_deref(), n.kind == NodeKind::Dir) {
            *file_score.entry(f).or_insert(0) += n.degree;
        }
    }
    let is_symbol =
        |k: NodeKind| matches!(k, NodeKind::Type | NodeKind::Function | NodeKind::Method);
    if !graph.communities.is_empty() {
        lines.push(String::new());
        lines.push("## Clusters (by size)".to_string());
        for c in graph.communities.iter().take(limits.clusters) {
            let mut members: Vec<&str> = graph
                .nodes
                .iter()
                .filter(|n| n.kind == NodeKind::File && n.community == c.id)
                .filter_map(|n| n.file.as_deref())
                .collect();
            members.sort_by(|a, b| {
                file_score
                    .get(b)
                    .unwrap_or(&0)
                    .cmp(file_score.get(a).unwrap_or(&0))
                    .then(a.cmp(b))
            });
            let prefix = format!("{}/", c.name);
            let key: Vec<&str> = members
                .iter()
                .take(4)
                .map(|f| f.strip_prefix(prefix.as_str()).unwrap_or(f))
                .collect();
            let mut hubs: Vec<&super::model::Node> = graph
                .nodes
                .iter()
                .filter(|n| n.community == c.id && is_symbol(n.kind) && n.degree > 0)
                .collect();
            hubs.sort_by(|a, b| b.degree.cmp(&a.degree).then(a.id.cmp(&b.id)));
            let hubs: Vec<&str> = hubs.iter().take(3).map(|n| n.name.as_str()).collect();
            let mut line = format!(
                "{}. {} ({} files): {}",
                c.id + 1,
                c.name,
                c.files,
                key.join(", ")
            );
            if !hubs.is_empty() {
                line.push_str(&format!("; hubs {}", hubs.join(", ")));
            }
            lines.push(line);
        }
        if graph.communities.len() > limits.clusters {
            lines.push(format!(
                "- and {} smaller clusters",
                graph.communities.len() - limits.clusters
            ));
        }
    }

    let mut hubs: Vec<&super::model::Node> = graph
        .nodes
        .iter()
        .filter(|n| is_symbol(n.kind) && n.degree > 0)
        .collect();
    hubs.sort_by(|a, b| b.degree.cmp(&a.degree).then(a.id.cmp(&b.id)));
    if !hubs.is_empty() {
        lines.push(String::new());
        lines.push("## Hubs (most linked)".to_string());
        for n in hubs.iter().take(limits.hubs) {
            lines.push(format!(
                "- {} ({}) {}:{}, {} links",
                n.name,
                n.kind.as_str(),
                n.file.as_deref().unwrap_or(""),
                n.line,
                n.degree
            ));
        }
    }
    cap(lines, limits.bytes)
}

/// `lines` joined, cut at a line so the text with `CUT_NOTE` fits in `bytes`.
fn cap(lines: Vec<String>, bytes: usize) -> String {
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        let room = bytes.saturating_sub(CUT_NOTE.len() + 1);
        if out.len() + line.len() + 1 > room && i + 1 < lines.len()
            || out.len() + line.len() + 1 > bytes
        {
            out.push_str(CUT_NOTE);
            return out;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::model::{Edge, EdgeKind, Node};

    fn node(id: &str, kind: NodeKind, file: &str, degree: u32) -> Node {
        Node {
            id: id.into(),
            kind,
            name: id
                .rsplit('#')
                .next()
                .unwrap()
                .rsplit('/')
                .next()
                .unwrap()
                .into(),
            file: Some(file.into()),
            line: 3,
            signature: String::new(),
            community: 0,
            degree,
            command: false,
        }
    }

    #[test]
    fn a_big_map_is_cut_at_a_line_under_the_budget() {
        let mut g = Graph::default();
        for i in 0..3000 {
            let f = format!("pkg{}/sub{}/file{i}.rs", i % 40, i % 7);
            g.nodes
                .push(node(&format!("file:{f}"), NodeKind::File, &f, 1));
            g.nodes.push(node(
                &format!("{f}#very_long_function_name_{i}"),
                NodeKind::Function,
                &f,
                i % 50,
            ));
        }
        g.edges.push(Edge {
            from: 0,
            to: 1,
            kind: EdgeKind::Calls,
            exact: true,
            line: 1,
        });
        crate::graph::community::assign(&mut g);
        let langs = BTreeMap::from([("rust".to_string(), 3000)]);
        let map = render(&g, &langs, PROMPT);
        assert!(map.len() <= PROMPT.bytes, "{}", map.len());
        assert!(map.ends_with(CUT_NOTE) || map.ends_with('\n'));
        let full = render(&g, &langs, FULL);
        assert!(full.len() <= FULL.bytes);
        assert!(full.len() > map.len());
        // The same graph gives the same bytes.
        assert_eq!(map, render(&g, &langs, PROMPT));
    }

    #[test]
    fn entry_points_name_mains_and_commands() {
        let mut g = Graph::default();
        g.nodes
            .push(node("file:src/main.rs", NodeKind::File, "src/main.rs", 0));
        g.nodes.push(node(
            "src/main.rs#main",
            NodeKind::Function,
            "src/main.rs",
            0,
        ));
        let mut cmd = node(
            "app/commands.rs#run_now",
            NodeKind::Function,
            "app/commands.rs",
            0,
        );
        cmd.command = true;
        g.nodes.push(cmd.clone());
        cmd.id = "app/commands.rs#stop".into();
        g.nodes.push(cmd);
        g.nodes.push(node(
            "file:ui/src/index.tsx",
            NodeKind::File,
            "ui/src/index.tsx",
            0,
        ));
        assert_eq!(
            entry_points(&g),
            vec![
                (
                    "app/commands.rs".to_string(),
                    "2 Tauri commands".to_string()
                ),
                ("src/main.rs".to_string(), "entry file, fn main".to_string()),
                ("ui/src/index.tsx".to_string(), "entry file".to_string()),
            ]
        );
    }
}
