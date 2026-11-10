//! Export in graphify's `graph.json` (project graph spec section 11): networkx's node-link
//! format, as graphify's query, path and viewer tools read it.

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use super::model::{EdgeKind, Graph, NodeKind};

/// What the export marks its file with, so a later export knows it may replace it.
const GENERATOR: &str = "dots";

/// graphify's id of a node: lower case, each run of other characters than letters and digits
/// one `_`.
fn graphify_id(id: &str) -> String {
    let mut out = String::new();
    for c in id.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

/// The graph as graphify's JSON.
pub fn to_graphify(graph: &Graph, commit: Option<&str>) -> Value {
    // Ids that the slug would make equal keep their own with the index appended.
    let mut used = std::collections::HashSet::new();
    let ids: Vec<String> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let id = graphify_id(&n.id);
            if used.insert(id.clone()) {
                id
            } else {
                let id = format!("{id}_{i}");
                used.insert(id.clone());
                id
            }
        })
        .collect();
    let nodes: Vec<Value> = graph
        .nodes
        .iter()
        .zip(&ids)
        .map(|(n, id)| {
            let label = match n.kind {
                NodeKind::Function | NodeKind::Method => format!("{}()", n.name),
                _ => n.name.clone(),
            };
            let file_type = match (n.kind, n.file.as_deref()) {
                (NodeKind::Heading, _) => "document",
                (_, Some(f)) if f.ends_with(".md") => "document",
                (NodeKind::Package, _) => "concept",
                _ => "code",
            };
            json!({
                "id": id,
                "label": label,
                "file_type": file_type,
                "source_file": n.file.clone().unwrap_or_default(),
                "source_location": format!("L{}", n.line.max(1)),
                "community": n.community,
                "norm_label": label.to_lowercase(),
                "_origin": GENERATOR,
            })
        })
        .collect();
    let links: Vec<Value> = graph
        .edges
        .iter()
        .map(|e| {
            let relation = match e.kind {
                EdgeKind::Contains => "contains",
                EdgeKind::Imports => "imports_from",
                EdgeKind::Calls => "calls",
                EdgeKind::Implements => "implements",
                EdgeKind::Extends => "extends",
                EdgeKind::Mentions => "references",
            };
            let from = &graph.nodes[e.from as usize];
            json!({
                "source": ids[e.from as usize],
                "target": ids[e.to as usize],
                "relation": relation,
                "confidence": if e.exact { "EXTRACTED" } else { "INFERRED" },
                "confidence_score": if e.exact { 1.0 } else { 0.6 },
                "source_file": from.file.clone().unwrap_or_default(),
                "source_location": format!("L{}", e.line.max(1)),
                "weight": 1.0,
                "_origin": GENERATOR,
            })
        })
        .collect();
    let mut out = json!({
        "directed": false,
        "multigraph": false,
        "graph": { "generator": GENERATOR },
        "nodes": nodes,
        "links": links,
        "hyperedges": [],
    });
    if let Some(c) = commit {
        out["built_at_commit"] = json!(c);
    }
    out
}

/// Writes the export to `<root>/graphify-out/graph.json`. A `graph.json` there that Dots did
/// not write is first renamed `graph.json.bak-<unix seconds>`. Returns the file written.
pub fn write(root: &Path, graph: &Graph, commit: Option<&str>) -> Result<PathBuf, String> {
    let dir = root.join("graphify-out");
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    let target = dir.join("graph.json");
    if target.is_file() && !written_by_dots(&target) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backup = dir.join(format!("graph.json.bak-{secs}"));
        std::fs::rename(&target, &backup)
            .map_err(|e| format!("cannot keep the existing graph.json: {e}"))?;
    }
    let text = serde_json::to_vec_pretty(&to_graphify(graph, commit))
        .map_err(|e| format!("cannot write the export: {e}"))?;
    let tmp = dir.join("graph.json.dots-tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("cannot write the export: {e}"))?;
    std::fs::rename(&tmp, &target).map_err(|e| format!("cannot write the export: {e}"))?;
    Ok(target)
}

/// Whether the `graph.json` at `path` was written by this export: its `graph.generator` is
/// `dots`. Only its head is read.
fn written_by_dots(path: &Path) -> bool {
    use std::io::Read;
    let mut head = String::new();
    let _ = std::fs::File::open(path).and_then(|f| f.take(4096).read_to_string(&mut head));
    head.contains("\"generator\": \"dots\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::model::{Edge, Node};

    fn graph() -> Graph {
        let node = |id: &str, kind, file: &str| Node {
            id: id.into(),
            kind,
            name: id.rsplit('#').next().unwrap().into(),
            file: Some(file.into()),
            line: 2,
            signature: String::new(),
            community: 1,
            degree: 1,
            command: false,
        };
        Graph {
            nodes: vec![
                node("file:src/a.rs", NodeKind::File, "src/a.rs"),
                node("src/a.rs#run", NodeKind::Function, "src/a.rs"),
            ],
            edges: vec![Edge {
                from: 0,
                to: 1,
                kind: EdgeKind::Contains,
                exact: true,
                line: 2,
            }],
            communities: Vec::new(),
        }
    }

    #[test]
    fn the_export_has_graphifys_shape() {
        let v = to_graphify(&graph(), Some("abc"));
        assert_eq!(v["directed"], false);
        assert_eq!(v["nodes"][1]["id"], "src_a_rs_run");
        assert_eq!(v["nodes"][1]["label"], "run()");
        assert_eq!(v["nodes"][1]["source_location"], "L2");
        assert_eq!(v["nodes"][1]["community"], 1);
        assert_eq!(v["links"][0]["source"], "file_src_a_rs");
        assert_eq!(v["links"][0]["relation"], "contains");
        assert_eq!(v["links"][0]["confidence"], "EXTRACTED");
        assert_eq!(v["built_at_commit"], "abc");
    }

    #[test]
    fn an_existing_graph_not_from_dots_is_kept_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("graphify-out");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("graph.json"), "{\"nodes\": []}").unwrap();
        let written = write(tmp.path(), &graph(), None).unwrap();
        assert!(written.is_file());
        let backups: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("graph.json.bak-")
            })
            .collect();
        assert_eq!(backups.len(), 1);
        // Exporting again replaces Dots' own file without another backup.
        write(tmp.path(), &graph(), None).unwrap();
        let count = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(count, 2);
    }
}
