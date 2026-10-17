//! A Claude Code plugin folder, read for the Library (MCP/skills spec section 2).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::skill::{read_skill, SkillInfo};
use crate::library::{plugin_server_name, LibraryConfig, McpServerConfig};
use crate::{Error, Result};

/// An MCP server of the plugin, as `.mcp.json` names it, with `${CLAUDE_PLUGIN_ROOT}` replaced
/// by the plugin's folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginServer {
    pub name: String,
    pub config: McpServerConfig,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PluginInfo {
    /// The manifest `name`, else the folder name. The caller checks it with `check_name`.
    pub name: String,
    pub root: PathBuf,
    pub version: Option<String>,
    pub description: Option<String>,
    /// `skills/<name>/SKILL.md`, by folder; or the plugin's own `SKILL.md` for a single-skill
    /// plugin. A skill in `skills/` that cannot be read is left out with a warning.
    pub skills: Vec<SkillInfo>,
    /// The root holds `SKILL.md` and there is no `skills/` folder.
    pub single_skill: bool,
    /// File names without `.md`, from `commands/` and `agents/`.
    pub commands: Vec<String>,
    pub agents: Vec<String>,
    /// `hooks/hooks.json` exists, or the manifest has `hooks`.
    pub has_hooks: bool,
    pub has_monitors: bool,
    pub has_bin: bool,
    pub servers: Vec<PluginServer>,
    /// What could not be read, in words for the person (an unreadable server, a bad manifest).
    pub warnings: Vec<String>,
}

impl PluginInfo {
    /// The plugin's skills with their qualified names, `<library name>:<skill name>`, the names
    /// Claude Code gives them in a run (the copy's manifest is named `library_name`) and the
    /// local engine's `skill` tool and prompt use.
    pub fn qualified_skills(&self, library_name: &str) -> Vec<(String, &SkillInfo)> {
        self.skills
            .iter()
            .map(|s| (format!("{library_name}:{}", s.name), s))
            .collect()
    }

    /// The servers as Library items would hold them: the name `plugin-<plugin>-<server>` (made
    /// valid) with the config. `plugin` is the plugin's Library name. A server whose name comes
    /// out the same as an earlier one's is left out.
    pub fn server_items(&self, plugin: &str) -> Vec<(String, McpServerConfig)> {
        let mut items: Vec<(String, McpServerConfig)> = Vec::new();
        for server in &self.servers {
            let name = plugin_server_name(plugin, &server.name);
            if !items.iter().any(|(n, _)| *n == name) {
                items.push((name, server.config.clone()));
            }
        }
        items
    }
}

fn read_json(path: &Path) -> std::result::Result<Option<Value>, String> {
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.to_string()),
    };
    serde_json::from_str(raw.trim_start_matches('\u{feff}'))
        .map(Some)
        .map_err(|e| e.to_string())
}

fn names_of_md(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".md").map(String::from)
        })
        .collect();
    names.sort();
    names
}

/// `value` with `${CLAUDE_PLUGIN_ROOT}` replaced in every string.
fn with_root(value: &Value, root: &str) -> Value {
    match value {
        Value::String(s) => Value::String(s.replace("${CLAUDE_PLUGIN_ROOT}", root)),
        Value::Array(a) => Value::Array(a.iter().map(|v| with_root(v, root)).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), with_root(v, root)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// One entry of an `mcpServers` object as a Library config: `type` is `stdio`, `http`, `sse` or
/// `streamable-http`; without it a `url` means http and a `command` stdio.
fn server_config(entry: &Value) -> std::result::Result<McpServerConfig, String> {
    let obj = entry.as_object().ok_or("not an object")?;
    let text = |v: &Value| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let strings = |key: &str| -> BTreeMap<String, String> {
        obj.get(key)
            .and_then(Value::as_object)
            .map(|m| m.iter().map(|(k, v)| (k.clone(), text(v))).collect())
            .unwrap_or_default()
    };
    let default_type = if obj.contains_key("url") {
        "http"
    } else {
        "stdio"
    };
    let config = match obj
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(default_type)
    {
        "stdio" => McpServerConfig::Stdio {
            command: obj
                .get("command")
                .and_then(Value::as_str)
                .ok_or("no command")?
                .to_string(),
            args: obj
                .get("args")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(text).collect())
                .unwrap_or_default(),
            cwd: obj.get("cwd").and_then(Value::as_str).map(String::from),
            env: strings("env"),
        },
        "http" | "sse" | "streamable-http" => McpServerConfig::Http {
            url: obj
                .get("url")
                .and_then(Value::as_str)
                .ok_or("no url")?
                .to_string(),
            headers: strings("headers"),
        },
        other => return Err(format!("transport {other:?} is not supported")),
    };
    LibraryConfig::McpServer(config.clone())
        .validate()
        .map_err(|e| e.to_string())?;
    Ok(config)
}

/// The servers of an `.mcp.json` value: `{"mcpServers": {...}}` or the servers object itself.
fn servers_object(value: &Value) -> Option<&Map<String, Value>> {
    let obj = value.as_object()?;
    match obj.get("mcpServers") {
        Some(Value::Object(inner)) => Some(inner),
        _ => Some(obj),
    }
}

/// Adds the servers of an `.mcp.json` value, with `${CLAUDE_PLUGIN_ROOT}` replaced. A server
/// already listed (by name) is kept as it is.
fn add_servers(
    servers: &mut Vec<PluginServer>,
    warnings: &mut Vec<String>,
    root: &str,
    origin: &str,
    value: &Value,
) {
    let Some(map) = servers_object(value) else {
        warnings.push(format!("{origin}: not an object"));
        return;
    };
    for (server, entry) in map {
        match server_config(&with_root(entry, root)) {
            Ok(config) if !servers.iter().any(|s| s.name == *server) => {
                servers.push(PluginServer {
                    name: server.clone(),
                    config,
                })
            }
            Ok(_) => {}
            Err(e) => warnings.push(format!("MCP server {server} ({origin}): {e}")),
        }
    }
}

/// Reads the plugin folder `root`. `NotFound` when it is not a folder. Parts that cannot be
/// read are left out and named in `warnings`.
pub fn read_plugin(root: &Path) -> Result<PluginInfo> {
    if !root.is_dir() {
        return Err(Error::NotFound(format!(
            "folder not found: {}",
            root.display()
        )));
    }
    let mut warnings = Vec::new();
    let folder = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();

    let manifest = match read_json(&root.join(".claude-plugin").join("plugin.json")) {
        Ok(m) => m.filter(Value::is_object),
        Err(e) => {
            warnings.push(format!("plugin.json could not be read: {e}"));
            None
        }
    };
    let manifest_text = |key: &str| {
        manifest
            .as_ref()
            .and_then(|m| m.get(key))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
    };
    let name = manifest_text("name").unwrap_or(folder);

    let skills_dir = root.join("skills");
    let single_skill = root.join("SKILL.md").is_file() && !skills_dir.exists();
    let mut skills = Vec::new();
    if single_skill {
        match read_skill(root) {
            Ok(skill) => skills.push(skill),
            Err(e) => warnings.push(format!("SKILL.md could not be read: {e}")),
        }
    } else if let Ok(entries) = fs::read_dir(&skills_dir) {
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| e.path())
            .filter(|p| p.join("SKILL.md").is_file())
            .collect();
        dirs.sort();
        for dir in dirs {
            match read_skill(&dir) {
                Ok(skill) => skills.push(skill),
                Err(e) => warnings.push(format!("skill {}: {e}", dir.display())),
            }
        }
    }

    let root_text = root.to_string_lossy();
    let mut servers: Vec<PluginServer> = Vec::new();
    match read_json(&root.join(".mcp.json")) {
        Ok(Some(value)) => {
            add_servers(&mut servers, &mut warnings, &root_text, ".mcp.json", &value)
        }
        Ok(None) => {}
        Err(e) => warnings.push(format!(".mcp.json could not be read: {e}")),
    }
    match manifest.as_ref().and_then(|m| m.get("mcpServers")) {
        Some(value @ Value::Object(_)) => add_servers(
            &mut servers,
            &mut warnings,
            &root_text,
            "plugin.json",
            value,
        ),
        Some(_) => warnings.push("plugin.json: mcpServers given as a path is not read".into()),
        None => {}
    }

    Ok(PluginInfo {
        name,
        version: manifest_text("version"),
        description: manifest_text("description"),
        skills,
        single_skill,
        commands: names_of_md(&root.join("commands")),
        agents: names_of_md(&root.join("agents")),
        has_hooks: root.join("hooks").join("hooks.json").is_file()
            || manifest.as_ref().is_some_and(|m| m.get("hooks").is_some()),
        has_monitors: root.join("monitors").exists(),
        has_bin: root.join("bin").exists(),
        servers,
        root: root.to_path_buf(),
        warnings,
    })
}
