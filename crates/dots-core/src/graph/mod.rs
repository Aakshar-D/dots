//! The project graph (project graph spec): a code graph of each project the dots work in,
//! built by Dots with tree-sitter (no model, no tokens), kept up to date incrementally, offered
//! to runs as the read-only `graph` tool and a map in the prompt, and shown on the Graph screen.
//!
//! - `project`: what a project is, and the key of its graph file;
//! - `extract`: one file's facts; `link`: the graph from every file's facts;
//! - `community` and `map`: clusters, hubs, entry points and the map;
//! - `store`: the graph file; `build`: a build; `query`: the tool; `export`: graphify's format.
//!
//! `Graphs` is the service: one build per project at a time, progress as events, and the
//! loaded graphs the tool answers from.

pub mod build;
pub mod community;
pub mod export;
pub mod extract;
pub mod link;
pub mod map;
pub mod model;
pub mod project;
pub mod query;
pub mod store;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::events::{Bus, RuntimeEvent};
pub use model::{Community, EdgeKind, NodeKind};
pub use project::Project;
use query::Loaded;

/// The graph files' folder, under the data folder.
pub const DIR: &str = "graphs";
/// The policy name of the `graph` tool.
pub const POLICY_NAME: &str = "Graph";
/// The tool's name on the local engine; `mcp__dots__graph` on the Claude engine.
pub const TOOL: &str = "graph";
/// Over this many nodes the screen shows one node per community.
pub const VIEW_NODES: usize = 2_000;
/// The most edges a view carries.
const VIEW_EDGES: usize = 8_000;
/// Progress events during a build come at most this often.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// The tool's name on `engine`.
pub fn model_name(engine: crate::model::EngineKind) -> &'static str {
    match engine {
        crate::model::EngineKind::Claude => "mcp__dots__graph",
        crate::model::EngineKind::Local => TOOL,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphState {
    /// No graph yet.
    None,
    Building,
    Ready,
    /// The last build failed; an older graph may still be there (`built_at`).
    Failed,
}

/// A project's graph, as the screen and the events show it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphStatus {
    pub key: String,
    pub root: String,
    pub state: GraphState,
    pub built_at: Option<String>,
    pub files: u64,
    pub nodes: u64,
    pub edges: u64,
    pub skipped: u64,
    pub duration_ms: u64,
    /// While building: how far, 0 to 100.
    pub progress: Option<u8>,
    pub error: Option<String>,
}

/// A node as the screen draws it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewNode {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub community: u32,
    pub file: Option<String>,
    pub line: u32,
    /// The node's links, or for a community its node count: what sizes it.
    pub weight: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViewEdge {
    pub source: String,
    pub target: String,
    pub kind: String,
    pub weight: u32,
}

/// What the Graph screen draws: the whole graph, one community of it, or (over `VIEW_NODES`)
/// one node per community.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphView {
    pub status: GraphStatus,
    /// One node per community: `community:<id>`.
    pub collapsed: bool,
    /// The community shown, when one was asked for.
    pub community: Option<u32>,
    pub nodes: Vec<ViewNode>,
    pub edges: Vec<ViewEdge>,
    pub communities: Vec<Community>,
}

/// A group of a node's neighbours: `calls`, `called by`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NeighbourGroup {
    pub label: String,
    pub nodes: Vec<ViewNode>,
    pub more: u32,
}

/// One node with its neighbours, for the screen's side panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDetail {
    pub node: ViewNode,
    pub signature: String,
    pub community_name: String,
    pub groups: Vec<NeighbourGroup>,
}

#[derive(Default)]
struct Slot {
    building: bool,
    dirty: bool,
    progress: Option<u8>,
    last_emit: Option<Instant>,
}

struct Inner {
    dir: PathBuf,
    bus: Option<Bus>,
    slots: Mutex<HashMap<String, Slot>>,
    cache: Mutex<HashMap<String, (String, Arc<Loaded>)>>,
    /// Builds run one at a time across projects.
    one_at_a_time: tokio::sync::Semaphore,
    limits: build::Limits,
}

/// The projects' graphs. Cheap to clone.
#[derive(Clone)]
pub struct Graphs {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Graphs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Graphs")
            .field("dir", &self.inner.dir)
            .finish()
    }
}

/// `GraphView` and others' node.
fn view_node(n: &model::Node) -> ViewNode {
    ViewNode {
        id: n.id.clone(),
        label: n.name.clone(),
        kind: n.kind.as_str().to_string(),
        community: n.community,
        file: n.file.clone(),
        line: n.line,
        weight: n.degree,
    }
}

impl Graphs {
    /// Graphs kept in `dir`; `bus` hears of each change.
    pub fn new(dir: PathBuf, bus: Option<Bus>) -> Graphs {
        Graphs::with_limits(dir, bus, build::Limits::default())
    }

    pub fn with_limits(dir: PathBuf, bus: Option<Bus>, limits: build::Limits) -> Graphs {
        Graphs {
            inner: Arc::new(Inner {
                dir,
                bus,
                slots: Mutex::new(HashMap::new()),
                cache: Mutex::new(HashMap::new()),
                one_at_a_time: tokio::sync::Semaphore::new(1),
                limits,
            }),
        }
    }

    /// The graph file of `project`.
    pub fn file_of(&self, project: &Project) -> PathBuf {
        self.inner.dir.join(format!("{}.sqlite", project.key))
    }

    fn slot_state(&self, key: &str) -> (bool, Option<u8>) {
        let slots = self.inner.slots.lock().unwrap();
        slots
            .get(key)
            .map_or((false, None), |s| (s.building, s.progress))
    }

    /// Whether a build of `project` is running.
    pub fn is_building(&self, project: &Project) -> bool {
        self.slot_state(&project.key).0
    }

    /// The state of `project`'s graph. Reads its file.
    pub fn status(&self, project: &Project) -> GraphStatus {
        let meta =
            store::open_existing(&self.file_of(project)).and_then(|conn| store::read_meta(&conn));
        let (building, progress) = self.slot_state(&project.key);
        let meta = meta.unwrap_or_default();
        let state = if building {
            GraphState::Building
        } else if meta.error.is_some() {
            GraphState::Failed
        } else if meta.built_at.is_some() {
            GraphState::Ready
        } else {
            GraphState::None
        };
        GraphStatus {
            key: project.key.clone(),
            root: project.root.to_string_lossy().into_owned(),
            state,
            built_at: meta.built_at,
            files: meta.files,
            nodes: meta.nodes,
            edges: meta.edges,
            skipped: meta.skipped,
            duration_ms: meta.duration_ms,
            progress: if building {
                Some(progress.unwrap_or(0))
            } else {
                None
            },
            error: meta.error,
        }
    }

    /// Whether `project` has a built graph.
    pub fn has_graph(&self, project: &Project) -> bool {
        self.status(project).built_at.is_some()
    }

    fn emit(&self, project: &Project) {
        if let Some(bus) = &self.inner.bus {
            let _ = bus.send(RuntimeEvent::GraphUpdated {
                status: self.status(project),
            });
        }
    }

    /// Starts a build of `project` unless it has a graph or one is building.
    pub fn ensure(&self, project: &Project) {
        if !self.is_building(project) && !self.has_graph(project) {
            self.rebuild(project);
        }
    }

    /// Starts a build of `project` in the background. One already running finishes first,
    /// and one more build follows it.
    pub fn rebuild(&self, project: &Project) {
        {
            let mut slots = self.inner.slots.lock().unwrap();
            let slot = slots.entry(project.key.clone()).or_default();
            if slot.building {
                slot.dirty = true;
                return;
            }
            slot.building = true;
            slot.progress = Some(0);
        }
        self.emit(project);
        let this = self.clone();
        let project = project.clone();
        tokio::spawn(async move {
            loop {
                let permit = this.inner.one_at_a_time.acquire().await;
                let commit = commit_of(&project.root).await;
                let (root, db, limits) = (
                    project.root.clone(),
                    this.file_of(&project),
                    this.inner.limits,
                );
                let reporter = this.clone();
                let key = project.key.clone();
                let proj = project.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let progress = |done: usize, total: usize| {
                        reporter.progress(&proj, &key, done, total);
                    };
                    build::build(&root, &db, commit, limits, &progress)
                })
                .await
                .unwrap_or_else(|e| Err(format!("the build stopped: {e}")));
                drop(permit);
                match result {
                    Ok(report) => tracing::info!(
                        root = %project.root.display(),
                        files = report.files,
                        parsed = report.parsed,
                        ms = report.duration_ms,
                        "project graph built"
                    ),
                    Err(e) => {
                        tracing::warn!(root = %project.root.display(), "project graph failed: {e}");
                        if let Ok(conn) = store::open(&this.file_of(&project)) {
                            let _ = store::set_error(&conn, &project.root.to_string_lossy(), &e);
                        }
                    }
                }
                let again = {
                    let mut slots = this.inner.slots.lock().unwrap();
                    let slot = slots.entry(project.key.clone()).or_default();
                    if slot.dirty {
                        slot.dirty = false;
                        slot.progress = Some(0);
                        true
                    } else {
                        slot.building = false;
                        slot.progress = None;
                        false
                    }
                };
                this.emit(&project);
                if !again {
                    break;
                }
            }
        });
    }

    fn progress(&self, project: &Project, key: &str, done: usize, total: usize) {
        let pct = (done * 100).checked_div(total).unwrap_or(0).min(99) as u8;
        let send = {
            let mut slots = self.inner.slots.lock().unwrap();
            let slot = slots.entry(key.to_string()).or_default();
            slot.progress = Some(pct);
            let due = slot.last_emit.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY);
            if due {
                slot.last_emit = Some(Instant::now());
            }
            due
        };
        if send {
            self.emit(project);
        }
    }

    /// The loaded graph of `project`, read again when a build has written a new one.
    /// Blocking. `None` before the first build.
    pub fn loaded(&self, project: &Project) -> Option<Arc<Loaded>> {
        let conn = store::open_existing(&self.file_of(project))?;
        let built = store::read_meta(&conn)?.built_at?;
        if let Some((at, l)) = self.inner.cache.lock().unwrap().get(&project.key) {
            if *at == built {
                return Some(l.clone());
            }
        }
        let (graph, maps, meta) = store::load(&conn).ok()??;
        let loaded = Arc::new(Loaded::new(graph, maps, meta));
        let mut cache = self.inner.cache.lock().unwrap();
        // Only the latest few graphs stay in memory.
        if cache.len() >= 4 && !cache.contains_key(&project.key) {
            if let Some(k) = cache.keys().next().cloned() {
                cache.remove(&k);
            }
        }
        cache.insert(project.key.clone(), (built, loaded.clone()));
        Some(loaded)
    }

    /// Answers a `graph` call for `project` (spec section 8). Starts a build when there is no
    /// graph yet.
    pub async fn call(&self, project: &Project, input: &Value) -> Result<String, String> {
        let this = self.clone();
        let (project, input) = (project.clone(), input.clone());
        tokio::task::spawn_blocking(move || this.call_blocking(&project, &input))
            .await
            .unwrap_or_else(|e| Err(format!("reading the graph failed: {e}")))
    }

    fn call_blocking(&self, project: &Project, input: &Value) -> Result<String, String> {
        let (building, progress) = self.slot_state(&project.key);
        let Some(loaded) = self.loaded(project) else {
            if !building {
                self.rebuild(project);
            }
            return Ok(format!(
                "graph building, {}% done; read the files directly for now",
                progress.unwrap_or(0)
            ));
        };
        let answer = query::call(&loaded, input)?;
        Ok(if building {
            format!(
                "(the graph is being rebuilt, {}% done; this answer is from the last build)\n{answer}",
                progress.unwrap_or(0)
            )
        } else {
            answer
        })
    }

    /// The prompt's section for `project` (spec section 9), for the tool named `tool`.
    /// Blocking: it reads the graph file.
    pub fn prompt_section(&self, project: &Project, tool: &str) -> String {
        let map = store::open_existing(&self.file_of(project)).and_then(|conn| {
            store::read_meta(&conn)?.built_at?;
            store::load(&conn)
                .ok()
                .flatten()
                .map(|(_, maps, _)| maps.prompt)
        });
        match map {
            Some(map) => format!(
                "## Project graph\n\nDots keeps a code graph of this project. Use the tool \
                 `{tool}` (search, node, callers, callees, file, path, neighbors) before broad \
                 file reads or greps; node ids look like `path#Type.method`.\n\n{}\n",
                map.trim_end()
            ),
            None => format!(
                "## Project graph\n\nA code graph of this project is being built; the tool \
                 `{tool}` reports its progress and answers once it is ready.\n"
            ),
        }
    }

    /// What the screen draws (spec section 10): the whole graph up to `VIEW_NODES` nodes, else
    /// one node per community; or the nodes of `community`.
    pub fn view(&self, project: &Project, community: Option<u32>) -> GraphView {
        let status = self.status(project);
        let Some(l) = self.loaded(project) else {
            return GraphView {
                status,
                collapsed: false,
                community: None,
                nodes: Vec::new(),
                edges: Vec::new(),
                communities: Vec::new(),
            };
        };
        let g = &l.graph;
        let communities = g.communities.clone();
        if community.is_none() && g.nodes.len() > VIEW_NODES {
            let mut weights: HashMap<(u32, u32), u32> = HashMap::new();
            for e in &g.edges {
                if e.kind == EdgeKind::Contains {
                    continue;
                }
                let a = g.nodes[e.from as usize].community;
                let b = g.nodes[e.to as usize].community;
                if a != b {
                    *weights.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                }
            }
            let mut edges: Vec<ViewEdge> = weights
                .into_iter()
                .map(|((a, b), w)| ViewEdge {
                    source: format!("community:{a}"),
                    target: format!("community:{b}"),
                    kind: "links".into(),
                    weight: w,
                })
                .collect();
            edges.sort_by(|a, b| {
                b.weight
                    .cmp(&a.weight)
                    .then(a.source.cmp(&b.source))
                    .then(a.target.cmp(&b.target))
            });
            edges.truncate(VIEW_EDGES);
            let nodes = communities
                .iter()
                .map(|c| ViewNode {
                    id: format!("community:{}", c.id),
                    label: format!("{} ({} files)", c.name, c.files),
                    kind: "community".into(),
                    community: c.id,
                    file: None,
                    line: 0,
                    weight: c.nodes,
                })
                .collect();
            return GraphView {
                status,
                collapsed: true,
                community: None,
                nodes,
                edges,
                communities,
            };
        }
        let keep: Vec<bool> = g
            .nodes
            .iter()
            .map(|n| community.is_none_or(|c| n.community == c))
            .collect();
        let mut order: Vec<usize> = (0..g.nodes.len()).filter(|i| keep[*i]).collect();
        // A large community shows its most linked nodes.
        if order.len() > VIEW_NODES {
            order.sort_by(|a, b| {
                g.nodes[*b]
                    .degree
                    .cmp(&g.nodes[*a].degree)
                    .then(g.nodes[*a].id.cmp(&g.nodes[*b].id))
            });
            order.truncate(VIEW_NODES);
            order.sort();
        }
        let mut shown = vec![false; g.nodes.len()];
        for i in &order {
            shown[*i] = true;
        }
        let nodes = order.iter().map(|i| view_node(&g.nodes[*i])).collect();
        let edges = g
            .edges
            .iter()
            .filter(|e| shown[e.from as usize] && shown[e.to as usize])
            .take(VIEW_EDGES)
            .map(|e| ViewEdge {
                source: g.nodes[e.from as usize].id.clone(),
                target: g.nodes[e.to as usize].id.clone(),
                kind: e.kind.as_str().into(),
                weight: 1,
            })
            .collect();
        GraphView {
            status,
            collapsed: false,
            community,
            nodes,
            edges,
            communities,
        }
    }

    /// One node and its neighbours by edge kind, for the side panel.
    pub fn node(&self, project: &Project, id: &str) -> Option<NodeDetail> {
        let l = self.loaded(project)?;
        let i = l.find_id(id)?;
        let g = &l.graph;
        let mut groups: std::collections::BTreeMap<(u8, EdgeKind), Vec<u32>> = Default::default();
        for e in &g.edges {
            if e.from == i {
                groups.entry((0, e.kind)).or_default().push(e.to);
            } else if e.to == i {
                groups.entry((1, e.kind)).or_default().push(e.from);
            }
        }
        let groups = groups
            .into_iter()
            .map(|((dir, kind), mut list)| {
                list.sort_by(|a, b| g.nodes[*a as usize].id.cmp(&g.nodes[*b as usize].id));
                list.dedup();
                let more = list.len().saturating_sub(50) as u32;
                NeighbourGroup {
                    label: if dir == 0 {
                        kind.as_str().to_string()
                    } else {
                        kind.reverse_words().to_string()
                    },
                    nodes: list
                        .iter()
                        .take(50)
                        .map(|j| view_node(&g.nodes[*j as usize]))
                        .collect(),
                    more,
                }
            })
            .collect();
        let n = &g.nodes[i as usize];
        Some(NodeDetail {
            node: view_node(n),
            signature: n.signature.clone(),
            community_name: g
                .communities
                .get(n.community as usize)
                .map(|c| c.name.clone())
                .unwrap_or_default(),
            groups,
        })
    }

    /// The nodes matching `query`, best first, at most 50.
    pub fn search(&self, project: &Project, query: &str) -> Vec<ViewNode> {
        let Some(l) = self.loaded(project) else {
            return Vec::new();
        };
        l.search(query, None)
            .into_iter()
            .take(50)
            .map(|(i, _)| view_node(l.node(i)))
            .collect()
    }

    /// Writes graphify's `graph.json` into the project (spec section 11).
    pub fn export(&self, project: &Project) -> Result<PathBuf, String> {
        let l = self
            .loaded(project)
            .ok_or("this project has no graph yet: build it first")?;
        export::write(&project.root, &l.graph, l.meta.commit.as_deref())
    }

    /// The file of the node `id`, inside the project, when it exists.
    pub fn file_path(&self, project: &Project, id: &str) -> Option<PathBuf> {
        let l = self.loaded(project)?;
        let n = l.node(l.find_id(id)?);
        let rel = n.file.as_deref()?;
        let path = rel
            .split('/')
            .fold(project.root.clone(), |p, part| p.join(part));
        let inside = path
            .canonicalize()
            .ok()
            .zip(project.root.canonicalize().ok())
            .is_some_and(|(p, r)| p.starts_with(r));
        inside.then_some(path)
    }
}

/// The commit `root` is at, when it is a git checkout.
async fn commit_of(root: &Path) -> Option<String> {
    if !root.join(".git").exists() {
        return None;
    }
    crate::workspace::git(root, &["rev-parse", "HEAD"])
        .await
        .ok()
        .filter(|c| !c.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn wait_ready(graphs: &Graphs, project: &Project) -> GraphStatus {
        for _ in 0..500 {
            let s = graphs.status(project);
            if s.state != GraphState::Building && s.built_at.is_some()
                || s.state == GraphState::Failed
            {
                return s;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("the build did not finish");
    }

    #[tokio::test]
    async fn the_service_builds_answers_and_writes_the_prompt_section() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("src/lib.rs"),
            "pub fn start() { helper(); }\nfn helper() {}\n",
        )
        .unwrap();
        let (bus, mut rx) = tokio::sync::broadcast::channel(64);
        let graphs = Graphs::new(tmp.path().join("graphs"), Some(bus));
        let project = Project::at(root.clone());
        assert_eq!(graphs.status(&project).state, GraphState::None);
        // Before a build, the tool starts one and says so.
        let first = graphs
            .call(&project, &serde_json::json!({"action": "map"}))
            .await
            .unwrap();
        assert!(first.starts_with("graph building"), "{first}");
        let status = wait_ready(&graphs, &project).await;
        assert_eq!(status.state, GraphState::Ready);
        assert_eq!(status.files, 1);
        let callers = graphs
            .call(
                &project,
                &serde_json::json!({"action": "callers", "node": "helper"}),
            )
            .await
            .unwrap();
        assert!(callers.contains("src/lib.rs#start"), "{callers}");
        let section = graphs.prompt_section(&project, "graph");
        assert!(section.starts_with("## Project graph"), "{section}");
        assert!(section.contains("# Project map"), "{section}");
        assert!(section.len() < 7_000);
        // Events said it was building, then ready.
        let mut states = Vec::new();
        while let Ok(RuntimeEvent::GraphUpdated { status }) = rx.try_recv() {
            states.push(status.state);
        }
        assert_eq!(states.first(), Some(&GraphState::Building));
        assert_eq!(states.last(), Some(&GraphState::Ready));
        // The view and the side panel.
        let view = graphs.view(&project, None);
        assert!(!view.collapsed);
        assert!(view.nodes.iter().any(|n| n.id == "src/lib.rs#start"));
        let detail = graphs.node(&project, "src/lib.rs#start").unwrap();
        assert!(detail.groups.iter().any(|g| g.label == "calls"));
        assert_eq!(graphs.search(&project, "help")[0].id, "src/lib.rs#helper");
        assert!(graphs.file_path(&project, "src/lib.rs#start").is_some());
    }

    #[tokio::test]
    async fn a_rebuild_asked_for_during_a_build_runs_once_more() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.py"), "def a(): pass\n").unwrap();
        let graphs = Graphs::new(tmp.path().join("graphs"), None);
        let project = Project::at(root.clone());
        graphs.rebuild(&project);
        std::fs::write(root.join("b.py"), "def b(): pass\n").unwrap();
        graphs.rebuild(&project);
        // The second ask waits for the first build and runs after it.
        for _ in 0..500 {
            if !graphs.is_building(&project) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(graphs.status(&project).files, 2);
    }

    #[tokio::test]
    async fn a_collapsed_view_has_one_node_per_community() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        for d in 0..3 {
            std::fs::create_dir_all(root.join(format!("m{d}"))).unwrap();
            for f in 0..10 {
                let body: String = (0..80)
                    .map(|i| format!("def f{d}_{f}_{i}(): pass\n"))
                    .collect();
                std::fs::write(root.join(format!("m{d}/f{f}.py")), body).unwrap();
            }
        }
        let graphs = Graphs::new(tmp.path().join("graphs"), None);
        let project = Project::at(root.clone());
        graphs.rebuild(&project);
        let status = wait_ready(&graphs, &project).await;
        assert!(status.nodes as usize > VIEW_NODES);
        let view = graphs.view(&project, None);
        assert!(view.collapsed);
        assert_eq!(view.nodes.len(), view.communities.len());
        let one = graphs.view(&project, Some(0));
        assert!(!one.collapsed);
        assert!(one.nodes.iter().all(|n| n.community == 0));
    }
}
