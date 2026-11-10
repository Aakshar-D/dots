//! Communities (project graph spec section 7): Louvain over the files, deterministic, then
//! every other node takes a community from its file or its neighbours.

use std::collections::{BTreeMap, HashMap};

use super::model::{Community, EdgeKind, Graph, NodeKind};

/// The weight of two files sharing a folder.
const SAME_FOLDER: f64 = 0.2;
/// The most passes over the nodes on one level, and the most levels.
const MAX_PASSES: usize = 10;
const MAX_LEVELS: usize = 10;

/// One community per node of a weighted undirected graph given by `adj` (each edge listed
/// from both ends; a self-loop once). Deterministic: nodes are visited in order, and a tie
/// goes to the smaller community.
pub fn louvain(adj: &[Vec<(usize, f64)>]) -> Vec<usize> {
    let n = adj.len();
    let mut membership: Vec<usize> = (0..n).collect();
    let mut graph: Vec<Vec<(usize, f64)>> = adj.to_vec();
    for _ in 0..MAX_LEVELS {
        let (level, moved) = one_level(&graph);
        if !moved {
            break;
        }
        // Renumber the communities 0.. in order of first appearance.
        let mut renumber: HashMap<usize, usize> = HashMap::new();
        for c in &level {
            let next = renumber.len();
            renumber.entry(*c).or_insert(next);
        }
        for m in membership.iter_mut() {
            *m = renumber[&level[*m]];
        }
        // Aggregate: one node per community.
        let k = renumber.len();
        let mut agg: Vec<BTreeMap<usize, f64>> = vec![BTreeMap::new(); k];
        for (u, edges) in graph.iter().enumerate() {
            let cu = renumber[&level[u]];
            for (v, w) in edges {
                let cv = renumber[&level[*v]];
                *agg[cu].entry(cv).or_insert(0.0) += w;
            }
        }
        graph = agg.into_iter().map(|m| m.into_iter().collect()).collect();
        if k == graph.len() && k == level.len() {
            break;
        }
    }
    membership
}

/// One level of Louvain: each node's community after moving nodes until none gains.
fn one_level(adj: &[Vec<(usize, f64)>]) -> (Vec<usize>, bool) {
    let n = adj.len();
    let degree: Vec<f64> = adj.iter().map(|e| e.iter().map(|(_, w)| w).sum()).collect();
    let m2: f64 = degree.iter().sum();
    let mut comm: Vec<usize> = (0..n).collect();
    let mut tot: Vec<f64> = degree.clone();
    let mut moved_any = false;
    if m2 == 0.0 {
        return (comm, false);
    }
    for _ in 0..MAX_PASSES {
        let mut moved = false;
        for u in 0..n {
            let cu = comm[u];
            // Weights from u to each neighbouring community.
            let mut to: BTreeMap<usize, f64> = BTreeMap::new();
            for (v, w) in &adj[u] {
                if *v != u {
                    *to.entry(comm[*v]).or_insert(0.0) += w;
                }
            }
            tot[cu] -= degree[u];
            let gain = |c: usize, k_in: f64| k_in - tot[c] * degree[u] / m2;
            let mut best = cu;
            let mut best_gain = gain(cu, to.get(&cu).copied().unwrap_or(0.0));
            for (c, k_in) in &to {
                let g = gain(*c, *k_in);
                if g > best_gain + 1e-12 || ((g - best_gain).abs() <= 1e-12 && *c < best) {
                    best = *c;
                    best_gain = g;
                }
            }
            tot[best] += degree[u];
            if best != cu {
                comm[u] = best;
                moved = true;
                moved_any = true;
            }
        }
        if !moved {
            break;
        }
    }
    (comm, moved_any)
}

/// Sets each node's community and the graph's community list: Louvain over the files (links
/// between their symbols count for the files, and files sharing a folder are weakly linked);
/// symbols take their file's; folders and packages the most common among their neighbours.
/// Communities are numbered by size, largest first, ties by their first file's path.
pub fn assign(graph: &mut Graph) {
    let files: Vec<u32> = {
        let mut f: Vec<u32> = (0..graph.nodes.len() as u32)
            .filter(|i| graph.nodes[*i as usize].kind == NodeKind::File)
            .collect();
        f.sort_by(|a, b| {
            graph.nodes[*a as usize]
                .id
                .cmp(&graph.nodes[*b as usize].id)
        });
        f
    };
    // Each node's file, by its position in `files`; none for a folder or a package.
    let node_file: Vec<Option<usize>> = {
        let file_pos: HashMap<&str, usize> = files
            .iter()
            .enumerate()
            .map(|(i, f)| (graph.nodes[*f as usize].file.as_deref().unwrap_or(""), i))
            .collect();
        graph
            .nodes
            .iter()
            .map(|n| match n.kind {
                NodeKind::Dir | NodeKind::Package => None,
                _ => n.file.as_deref().and_then(|f| file_pos.get(f).copied()),
            })
            .collect()
    };
    let file_of = |node: u32| -> Option<usize> { node_file[node as usize] };
    let mut weights: BTreeMap<(usize, usize), f64> = BTreeMap::new();
    for e in &graph.edges {
        if e.kind == EdgeKind::Contains {
            continue;
        }
        if let (Some(a), Some(b)) = (file_of(e.from), file_of(e.to)) {
            if a != b {
                *weights.entry((a.min(b), a.max(b))).or_insert(0.0) += 1.0;
            }
        }
    }
    // Files in the same folder: a chain in path order is enough to hold them together.
    let mut by_dir: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, f) in files.iter().enumerate() {
        let path = graph.nodes[*f as usize].file.as_deref().unwrap_or("");
        let dir = path.rsplit_once('/').map_or("", |(d, _)| d);
        by_dir.entry(dir).or_default().push(i);
    }
    for members in by_dir.values() {
        for pair in members.windows(2) {
            *weights.entry((pair[0], pair[1])).or_insert(0.0) += SAME_FOLDER;
        }
    }
    let mut adj: Vec<Vec<(usize, f64)>> = vec![Vec::new(); files.len()];
    for ((a, b), w) in &weights {
        adj[*a].push((*b, *w));
        adj[*b].push((*a, *w));
    }
    let raw = louvain(&adj);

    // Number by size, largest first, ties by first file.
    let mut sizes: BTreeMap<usize, (usize, usize)> = BTreeMap::new();
    for (i, c) in raw.iter().enumerate() {
        let e = sizes.entry(*c).or_insert((0, i));
        e.0 += 1;
    }
    let mut order: Vec<(usize, usize, usize)> = sizes
        .iter()
        .map(|(c, (n, first))| (*c, *n, *first))
        .collect();
    order.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
    let number: HashMap<usize, u32> = order
        .iter()
        .enumerate()
        .map(|(i, (c, _, _))| (*c, i as u32))
        .collect();
    let file_comm: Vec<u32> = raw.iter().map(|c| number[c]).collect();

    // Symbols and files.
    let mut unset = Vec::new();
    for i in 0..graph.nodes.len() {
        match file_of(i as u32) {
            Some(f) => graph.nodes[i].community = file_comm[f],
            None => unset.push(i),
        }
    }
    // Folders and packages: the most common community of their neighbours.
    let mut votes: HashMap<usize, BTreeMap<u32, usize>> = HashMap::new();
    for e in &graph.edges {
        for (a, b) in [(e.from, e.to), (e.to, e.from)] {
            let kind = graph.nodes[a as usize].kind;
            if matches!(kind, NodeKind::Dir | NodeKind::Package) {
                if let Some(f) = file_of(b) {
                    *votes
                        .entry(a as usize)
                        .or_default()
                        .entry(file_comm[f])
                        .or_insert(0) += 1;
                }
            }
        }
    }
    for i in unset {
        let c = votes
            .get(&i)
            .and_then(|v| v.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))))
            .map(|(c, _)| *c)
            .unwrap_or(0);
        graph.nodes[i].community = c;
    }

    // The list, each named after the folder most of its files share.
    let mut list: Vec<Community> = Vec::new();
    for (i, _) in order.iter().enumerate() {
        list.push(Community {
            id: i as u32,
            name: String::new(),
            nodes: 0,
            files: 0,
        });
    }
    // Folders are counted twice: of the source files, and of all (tests and docs too). A name
    // comes from the source files when the community has any.
    let mut dirs: Vec<(BTreeMap<&str, usize>, BTreeMap<&str, usize>)> =
        vec![(BTreeMap::new(), BTreeMap::new()); list.len()];
    for (i, f) in files.iter().enumerate() {
        let c = file_comm[i] as usize;
        list[c].files += 1;
        let path = graph.nodes[*f as usize].file.as_deref().unwrap_or("");
        let dir = path.rsplit_once('/').map_or(".", |(d, _)| d);
        *dirs[c].1.entry(dir).or_insert(0) += 1;
        if !is_test_or_doc(path) {
            *dirs[c].0.entry(dir).or_insert(0) += 1;
        }
    }
    for n in &graph.nodes {
        if let Some(c) = list.get_mut(n.community as usize) {
            c.nodes += 1;
        }
    }
    // Each the folder most of its files share, largest community first; a folder a larger one
    // took gives way to the next, so two clusters are not named alike.
    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (c, (source, all)) in list.iter_mut().zip(dirs) {
        let counts = if source.is_empty() { all } else { source };
        let mut ranked: Vec<(&str, usize)> = counts.into_iter().collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        let name = ranked
            .iter()
            .map(|(d, _)| d.to_string())
            .find(|d| !used.contains(d))
            .unwrap_or_else(|| {
                let first = ranked.first().map_or(".", |(d, _)| d);
                format!("{first} ({})", c.id + 1)
            });
        used.insert(name.clone());
        c.name = name;
    }
    graph.communities = list;
}

/// A test or a document: what a cluster is not named after when it has other files.
fn is_test_or_doc(path: &str) -> bool {
    let p = format!("/{path}");
    p.contains("/tests/")
        || p.contains("/test/")
        || p.contains("/__tests__/")
        || p.contains(".test.")
        || p.contains("_test.")
        || p.ends_with(".md")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adj(n: usize, edges: &[(usize, usize)]) -> Vec<Vec<(usize, f64)>> {
        let mut a = vec![Vec::new(); n];
        for (x, y) in edges {
            a[*x].push((*y, 1.0));
            a[*y].push((*x, 1.0));
        }
        a
    }

    #[test]
    fn two_cliques_joined_by_one_edge_are_two_communities() {
        let edges = [(0, 1), (0, 2), (1, 2), (3, 4), (3, 5), (4, 5), (2, 3)];
        let c = louvain(&adj(6, &edges));
        assert_eq!(c[0], c[1]);
        assert_eq!(c[1], c[2]);
        assert_eq!(c[3], c[4]);
        assert_eq!(c[4], c[5]);
        assert_ne!(c[0], c[3]);
        // Deterministic.
        assert_eq!(c, louvain(&adj(6, &edges)));
    }

    #[test]
    fn isolated_nodes_keep_their_own_community() {
        let c = louvain(&adj(3, &[]));
        assert_eq!(c, vec![0, 1, 2]);
    }
}
