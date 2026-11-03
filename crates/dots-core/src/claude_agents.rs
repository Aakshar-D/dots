//! Claude Code agents found on this PC, and the dots they become (import agents spec): the scan
//! of the user's, the projects' and the installed plugins' agent folders, and the mapping of one
//! agent file to a `DotSpec`. The runtime re-scans and creates the dots.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::kit::{agent_files, agent_model, agent_policy, dot_name, read_agent, AgentFile};
use crate::model::{Dot, DotSpec, WorkspaceMode};

/// Where an agent file was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSourceKind {
    /// `~/.claude/agents`
    User,
    /// `<workdir>/.claude/agents` of a dot's workdir.
    Project,
    /// `agents/` of an installed Claude Code plugin.
    Plugin,
}

/// One agent file as the import dialog lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScannedAgent {
    pub source: AgentSourceKind,
    /// The plugin's name, for a plugin agent.
    pub plugin: Option<String>,
    /// The project folder, for a project agent.
    pub project: Option<String>,
    /// The file; what `import_claude_agents` takes back.
    pub path: String,
    /// The dot name it would get (before a number is added to a taken one).
    pub name: String,
    pub description: Option<String>,
    /// The `model` as the file writes it.
    pub model: Option<String>,
    /// The `tools` the file lists; `None` when it lists none (it has them all).
    pub tools: Option<Vec<String>>,
    /// What the person should know: tools left out, tools that ask.
    pub warnings: Vec<String>,
    /// A dot has this name already (ignoring case).
    pub already_imported: bool,
    /// Why the file cannot be imported; `None` when it can.
    pub error: Option<String>,
}

/// An agent that was not imported, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportFailure {
    pub path: String,
    pub reason: String,
}

/// What an import did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentImport {
    pub created: Vec<Dot>,
    pub failed: Vec<ImportFailure>,
}

/// The folders a scan reads.
#[derive(Debug, Clone, Default)]
pub struct AgentSources {
    /// `~/.claude/agents`, when the home folder is known.
    pub user: Option<PathBuf>,
    /// Each project folder (a dot's workdir) and its workspace mode.
    pub projects: Vec<(PathBuf, WorkspaceMode)>,
    /// Each installed plugin's name and folder.
    pub plugins: Vec<(String, PathBuf)>,
}

/// One file of a scan, with what is needed to make its dot.
#[derive(Debug, Clone)]
pub struct Found {
    pub listed: ScannedAgent,
    pub agent: Option<AgentFile>,
    /// The project folder and its mode, for a project agent.
    pub project: Option<(PathBuf, WorkspaceMode)>,
}

fn entry(
    path: &Path,
    source: AgentSourceKind,
    plugin: Option<&str>,
    project: Option<&(PathBuf, WorkspaceMode)>,
    taken: &BTreeSet<String>,
) -> Found {
    let read = read_agent(path);
    let mut listed = ScannedAgent {
        source,
        plugin: plugin.map(String::from),
        project: project.map(|(p, _)| p.to_string_lossy().into_owned()),
        path: path.to_string_lossy().into_owned(),
        name: String::new(),
        description: None,
        model: None,
        tools: None,
        warnings: Vec::new(),
        already_imported: false,
        error: None,
    };
    match &read {
        Ok(agent) => {
            listed.name = dot_name(&agent.name);
            listed.description = agent.description.clone();
            listed.model = agent.model.clone();
            listed.tools = agent.tools.clone();
            listed.warnings = agent_policy(agent.tools.as_deref()).1;
            listed.already_imported = taken.contains(&listed.name.to_lowercase());
        }
        Err(reason) => {
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned());
            listed.name = dot_name(&stem.unwrap_or_default());
            listed.error = Some(reason.clone());
        }
    }
    Found {
        listed,
        agent: read.ok(),
        project: project.cloned(),
    }
}

/// Every agent file of `sources`, user first, then projects, then plugins. A file found twice
/// (a project folder that is the home folder) is listed once, where it was found first.
/// `dot_names` are the names of the dots there are.
pub fn scan(sources: &AgentSources, dot_names: &[String]) -> Vec<Found> {
    let taken: BTreeSet<String> = dot_names.iter().map(|n| n.to_lowercase()).collect();
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    let mut add = |path: PathBuf, found: &mut dyn FnMut(&Path) -> Found| {
        let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if seen.insert(key) {
            out.push(found(&path));
        }
    };
    if let Some(dir) = &sources.user {
        for path in agent_files(dir) {
            add(path, &mut |p| {
                entry(p, AgentSourceKind::User, None, None, &taken)
            });
        }
    }
    for project in &sources.projects {
        let dir = project.0.join(".claude").join("agents");
        for path in agent_files(&dir) {
            add(path, &mut |p| {
                entry(p, AgentSourceKind::Project, None, Some(project), &taken)
            });
        }
    }
    for (name, root) in &sources.plugins {
        for path in agent_files(&root.join("agents")) {
            add(path, &mut |p| {
                entry(p, AgentSourceKind::Plugin, Some(name), None, &taken)
            });
        }
    }
    out
}

/// `name`, or `name-2`, `name-3`, … : the first not in `taken` (lower case), which it joins.
pub fn free_name(name: &str, taken: &mut BTreeSet<String>) -> String {
    let mut pick = name.to_string();
    let mut n = 2;
    while taken.contains(&pick.to_lowercase()) {
        let suffix = format!("-{n}");
        let keep = name.len().min(64 - suffix.len());
        pick = format!("{}{suffix}", name[..keep].trim_end_matches('-'));
        n += 1;
    }
    taken.insert(pick.to_lowercase());
    pick
}

/// The dot an agent becomes, named `name`, working in `workdir` with `mode`.
pub fn agent_spec(agent: &AgentFile, name: &str, workdir: &str, mode: WorkspaceMode) -> DotSpec {
    let mut spec = DotSpec::new(name, &agent.prompt, workdir);
    if let Some(model) = agent_model(agent.model.as_deref()) {
        spec.model = model;
    }
    spec.workspace_mode = mode;
    spec.policy = agent_policy(agent.tools.as_deref()).0;
    spec
}
