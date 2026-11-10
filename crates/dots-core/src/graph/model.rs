//! The graph as the builder writes it and the tool and the screen read it (project graph spec
//! section 4).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Dir,
    File,
    Type,
    Function,
    Method,
    Heading,
    Package,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Dir => "dir",
            NodeKind::File => "file",
            NodeKind::Type => "type",
            NodeKind::Function => "function",
            NodeKind::Method => "method",
            NodeKind::Heading => "heading",
            NodeKind::Package => "package",
        }
    }

    pub fn parse(s: &str) -> Option<NodeKind> {
        Some(match s {
            "dir" => NodeKind::Dir,
            "file" => NodeKind::File,
            "type" => NodeKind::Type,
            "function" => NodeKind::Function,
            "method" => NodeKind::Method,
            "heading" => NodeKind::Heading,
            "package" => NodeKind::Package,
            _ => return None,
        })
    }

    /// A function or a method: what calls go between.
    pub fn callable(self) -> bool {
        matches!(self, NodeKind::Function | NodeKind::Method)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeKind {
    Contains,
    Imports,
    Calls,
    Implements,
    Extends,
    Mentions,
}

impl EdgeKind {
    pub const ALL: [EdgeKind; 6] = [
        EdgeKind::Contains,
        EdgeKind::Imports,
        EdgeKind::Calls,
        EdgeKind::Implements,
        EdgeKind::Extends,
        EdgeKind::Mentions,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Contains => "contains",
            EdgeKind::Imports => "imports",
            EdgeKind::Calls => "calls",
            EdgeKind::Implements => "implements",
            EdgeKind::Extends => "extends",
            EdgeKind::Mentions => "mentions",
        }
    }

    pub fn parse(s: &str) -> Option<EdgeKind> {
        EdgeKind::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// The words for the edge read backwards: `called by`, `imported by`.
    pub fn reverse_words(self) -> &'static str {
        match self {
            EdgeKind::Contains => "contained in",
            EdgeKind::Imports => "imported by",
            EdgeKind::Calls => "called by",
            EdgeKind::Implements => "implemented by",
            EdgeKind::Extends => "extended by",
            EdgeKind::Mentions => "mentioned in",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub name: String,
    /// The file, relative to the root with `/`; `None` for a package.
    pub file: Option<String>,
    /// 1-based; 0 for a folder, a package or a whole file.
    pub line: u32,
    pub signature: String,
    pub community: u32,
    /// Links other than `contains`, both ways.
    pub degree: u32,
    /// A Rust function marked `#[tauri::command]`.
    pub command: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub from: u32,
    pub to: u32,
    pub kind: EdgeKind,
    /// `false` for a link matched by name only (`inferred`).
    pub exact: bool,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Community {
    pub id: u32,
    /// The folder most of its files share.
    pub name: String,
    pub nodes: u32,
    pub files: u32,
}

/// A built graph, in memory.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub communities: Vec<Community>,
}

impl Graph {
    /// The index of each node by id.
    pub fn index(&self) -> HashMap<&str, u32> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i as u32))
            .collect()
    }

    /// Sets each node's degree from the edges other than `contains`.
    pub fn count_degrees(&mut self) {
        for n in &mut self.nodes {
            n.degree = 0;
        }
        for e in &self.edges {
            if e.kind == EdgeKind::Contains || e.from == e.to {
                continue;
            }
            self.nodes[e.from as usize].degree += 1;
            self.nodes[e.to as usize].degree += 1;
        }
    }
}
