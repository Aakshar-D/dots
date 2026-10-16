//! The plugins Claude Code has installed, from `installed_plugins.json`. The file's schema is
//! not documented, so this reads what it can and ignores the rest.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPlugin {
    /// `<name>@<marketplace>`
    pub id: String,
    pub name: String,
    pub marketplace: Option<String>,
    pub scope: Option<String>,
    pub install_path: PathBuf,
    pub version: Option<String>,
    /// `install_path` is a folder now (an update may have moved it).
    pub exists: bool,
}

/// The folder that holds `installed_plugins.json`: `CLAUDE_CODE_PLUGIN_CACHE_DIR` when set,
/// else `~/.claude/plugins` (the home folder from `USERPROFILE`, then `HOME`).
pub fn plugins_root(env: &dyn Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let set = |k: &str| env(k).filter(|v| !v.trim().is_empty());
    if let Some(dir) = set("CLAUDE_CODE_PLUGIN_CACHE_DIR") {
        return Some(PathBuf::from(dir));
    }
    let home = set("USERPROFILE").or_else(|| set("HOME"))?;
    Some(PathBuf::from(home).join(".claude").join("plugins"))
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// One plugin id's record, or the list of its records (one per scope). The user's scope wins.
fn plugin_from(id: &str, value: &Value) -> Option<InstalledPlugin> {
    let records: Vec<&Value> = match value {
        Value::Array(list) => list.iter().collect(),
        other => vec![other],
    };
    let readable: Vec<&Value> = records
        .into_iter()
        .filter(|r| text(r, "installPath").is_some())
        .collect();
    let record = readable
        .iter()
        .find(|r| r.get("scope").and_then(Value::as_str) == Some("user"))
        .or_else(|| readable.first())?;
    let install_path = PathBuf::from(text(record, "installPath")?);
    let (name, marketplace) = match id.split_once('@') {
        Some((n, m)) => (n.to_string(), Some(m.to_string())),
        None => (id.to_string(), None),
    };
    Some(InstalledPlugin {
        id: id.to_string(),
        name,
        marketplace,
        scope: text(record, "scope"),
        exists: install_path.is_dir(),
        install_path,
        version: text(record, "version"),
    })
}

/// The plugins listed in `<root>/installed_plugins.json`, by id. A missing or unreadable file
/// gives none. The records are in `plugins` (the shape seen so far) or, failing that, at the top
/// level.
pub fn read_installed(root: &Path) -> Vec<InstalledPlugin> {
    let Ok(raw) = fs::read_to_string(root.join("installed_plugins.json")) else {
        return Vec::new();
    };
    let Ok(Value::Object(top)) = serde_json::from_str::<Value>(&raw) else {
        return Vec::new();
    };
    let map = match top.get("plugins") {
        Some(Value::Object(plugins)) => plugins,
        _ => &top,
    };
    let mut found: Vec<InstalledPlugin> = map
        .iter()
        .filter_map(|(id, value)| plugin_from(id, value))
        .collect();
    found.sort_by(|a, b| a.id.cmp(&b.id));
    found
}

/// The installed plugins, from the folder `plugins_root` names.
pub fn installed_plugins(env: &dyn Fn(&str) -> Option<String>) -> Vec<InstalledPlugin> {
    plugins_root(env).map_or_else(Vec::new, |root| read_installed(&root))
}

/// The installed plugin with this id, if the file still lists it.
pub fn find_installed(env: &dyn Fn(&str) -> Option<String>, id: &str) -> Option<InstalledPlugin> {
    installed_plugins(env).into_iter().find(|p| p.id == id)
}
