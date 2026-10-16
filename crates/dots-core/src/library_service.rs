//! The Library at runtime (MCP/skills spec sections 3 and 9): what the app's `library_*`
//! commands do. It adds, changes and removes items (asking first when dots use one), tests an
//! MCP server config, checks a server and accepts its list with the fingerprint guard, keeps a
//! plugin's own MCP server items in line with its folder, scans a folder for skills, lists the
//! installed Claude Code plugins, and says which variables are set. Every change of an item or
//! a catalog is published as `RuntimeEvent::LibraryUpdated`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::events::{Bus, RuntimeEvent};
use crate::kit::{self, InstalledPlugin, PluginInfo};
use crate::library::{
    check_name, LibraryConfig, LibraryItem, LibraryKind, McpCatalog, McpServerConfig, PluginConfig,
};
use crate::mcp::{fingerprint, McpError, McpSession, McpTool, SessionOptions};
use crate::model::{DotRef, ToolsChanged};
use crate::store::Store;
use crate::{Error, Result};

/// Dots' own environment, where `${VAR}` is read.
pub(crate) fn process_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// One Library item as the Library screen shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LibraryEntry {
    #[serde(flatten)]
    pub item: LibraryItem,
    /// The dots that use it; for a plugin's MCP server, the dots that use the plugin.
    pub used_by: Vec<DotRef>,
    /// An MCP server's catalog.
    pub catalog: Option<McpCatalog>,
    /// The `${VAR}` of an MCP server's config that Dots' environment does not set.
    pub missing_variables: Vec<String>,
    /// A skill read from its folder.
    pub skill: Option<SkillDetail>,
    /// A plugin read from its folder.
    pub plugin: Option<PluginDetail>,
    /// Why a skill or plugin folder could not be read ("folder not found").
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillDetail {
    pub description: String,
}

/// A plugin's parts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PluginDetail {
    pub version: Option<String>,
    pub description: Option<String>,
    /// Qualified: `<plugin>:<skill>`.
    pub skills: Vec<String>,
    pub commands: Vec<String>,
    pub agents: Vec<String>,
    /// The names of its MCP server items.
    pub servers: Vec<String>,
    pub has_hooks: bool,
    /// What could not be read in it.
    pub warnings: Vec<String>,
}

/// What a server config lists (`library_probe`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct McpProbe {
    pub tools: Vec<McpTool>,
    pub fingerprint: String,
}

/// A skill found by `library_scan_skills`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScannedSkill {
    pub name: String,
    pub description: String,
    pub path: String,
    /// Why `name` cannot be a Library name; the person picks another.
    pub name_error: Option<String>,
}

/// Whether a variable is set in Dots' environment. Never its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnvVariable {
    pub name: String,
    pub set: bool,
}

pub(crate) fn publish(bus: &Bus, item_id: &str) {
    let _ = bus.send(RuntimeEvent::LibraryUpdated {
        item_id: item_id.to_string(),
    });
}

/// Opens a session with the server and lists its tools. The session is closed again when the
/// listing fails.
pub(crate) async fn open_and_list(
    name: &str,
    config: &McpServerConfig,
    options: SessionOptions,
) -> std::result::Result<(McpSession, Vec<McpTool>), McpError> {
    let session = McpSession::open(name, config, &process_env, options).await?;
    match session.list_tools().await {
        Ok(tools) => Ok((session, tools)),
        Err(e) => {
            session.close().await;
            Err(e)
        }
    }
}

/// Writes the outcome of a check to the server's catalog and publishes the change.
pub(crate) async fn record_check(
    store: &Store,
    bus: &Bus,
    server_id: &str,
    outcome: std::result::Result<&[McpTool], &McpError>,
) -> Result<McpCatalog> {
    let catalog = match outcome {
        Ok(tools) => {
            store
                .record_catalog_seen(
                    server_id,
                    &fingerprint(tools),
                    &serde_json::to_value(tools)?,
                )
                .await?
        }
        Err(e) => {
            store
                .record_catalog_failure(server_id, &e.to_string())
                .await?
        }
    };
    publish(bus, server_id);
    Ok(catalog)
}

/// Why the skill folder `dir` cannot be used, in the words of a run event.
pub(crate) fn skill_problem(dir: &Path, e: &Error) -> String {
    if !dir.is_dir() {
        "folder not found".into()
    } else if !dir.join("SKILL.md").is_file() {
        "SKILL.md not found".into()
    } else {
        format!("could not be read: {e}")
    }
}

fn plugin_problem(dir: &Path, e: &Error) -> String {
    if !dir.is_dir() {
        "folder not found".into()
    } else {
        format!("could not be read: {e}")
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Other(format!("reading the folder failed: {e}")))
}

fn plugin_config(item: &LibraryItem) -> Result<&PluginConfig> {
    match &item.config {
        LibraryConfig::Plugin(c) => Ok(c),
        _ => Err(Error::Invalid(format!("{:?} is not a plugin", item.name))),
    }
}

/// Serialises `sync_plugin_servers` (one lock for all plugins, which are few and synced rarely).
static PLUGIN_SYNC: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The reason given when a plugin leaves the Library while a run is opening it.
const GONE: &str = "removed from the Library";

/// Brings the plugin's MCP server items in line with what its folder lists (spec section 2):
/// new servers are added, changed ones updated (their catalog goes back to `unchecked`),
/// servers no longer listed removed. A server whose item name the Library already has for a
/// server of its own is left out, with a warning.
pub(crate) async fn sync_plugin_servers(
    store: &Store,
    bus: &Bus,
    plugin: &LibraryItem,
    info: &PluginInfo,
) -> Result<Vec<String>> {
    // One sync at a time for all plugins: two runs opening the same plugin must not both act on
    // the same stale list of owned items.
    let _turn = PLUGIN_SYNC.lock().await;
    // The plugin may have been removed since it was read: then there is nothing to keep in line.
    store.get_library_item(&plugin.id).await?;
    let mut warnings = Vec::new();
    let wanted = info.server_items(&plugin.name);
    let existing = store.owned_library_items(&plugin.id).await?;
    for old in &existing {
        if !wanted.iter().any(|(name, _)| *name == old.name) {
            match store.remove_library_item(&old.id).await {
                Ok(()) | Err(Error::NotFound(_)) => publish(bus, &old.id),
                Err(e) => return Err(e),
            }
        }
    }
    for (name, config) in wanted {
        let config = LibraryConfig::McpServer(config);
        match existing.iter().find(|i| i.name == name) {
            Some(old) if old.config == config => {}
            Some(old) => match store.update_library_item(&old.id, &config).await {
                Ok(_) | Err(Error::NotFound(_)) => publish(bus, &old.id),
                Err(e) => return Err(e),
            },
            None => match store
                .add_library_item(&name, &config, Some(&plugin.id))
                .await
            {
                Ok(item) => publish(bus, &item.id),
                Err(Error::Conflict(_)) => {
                    let ours = store
                        .find_library_item(LibraryKind::McpServer, &name)
                        .await?
                        .is_some_and(|i| i.owner_id.as_deref() == Some(plugin.id.as_str()));
                    if !ours {
                        warnings.push(format!(
                            "its MCP server {name} is left out: the Library already has an MCP \
                             server of that name"
                        ));
                    }
                }
                Err(e) => return Err(e),
            },
        }
    }
    Ok(warnings)
}

/// Reads the plugin's folder again, as a run does at its start (spec section 6). An installed
/// plugin whose folder is gone is looked up in Claude Code's list again, and its new folder is
/// stored. Its MCP server items are brought in line. `Ok(Err(reason))` when the folder cannot
/// be used: "folder not found" or "could not be read: ...".
pub(crate) async fn refresh_plugin(
    store: &Store,
    bus: &Bus,
    item: &LibraryItem,
) -> Result<std::result::Result<(PluginInfo, Vec<String>), String>> {
    let mut config = plugin_config(item)?.clone();
    if !Path::new(&config.path).is_dir() {
        let moved = config.installed_id.clone().and_then(|id| {
            kit::find_installed(&process_env, &id)
                .filter(|p| p.exists)
                .map(|p| p.install_path)
        });
        if let Some(path) = moved {
            config.path = path.to_string_lossy().into_owned();
            match store
                .update_library_item(&item.id, &LibraryConfig::Plugin(config.clone()))
                .await
            {
                Ok(_) => publish(bus, &item.id),
                Err(Error::NotFound(_)) => return Ok(Err(GONE.into())),
                Err(e) => return Err(e),
            }
        }
    }
    let root = PathBuf::from(&config.path);
    let read = blocking(move || {
        let info = kit::read_plugin(&root);
        info.map_err(|e| plugin_problem(&root, &e))
    })
    .await?;
    let info = match read {
        Ok(info) => info,
        Err(reason) => return Ok(Err(reason)),
    };
    let mut warnings = info.warnings.clone();
    match sync_plugin_servers(store, bus, item, &info).await {
        Ok(more) => warnings.extend(more),
        Err(Error::NotFound(_)) => return Ok(Err(GONE.into())),
        Err(e) => return Err(e),
    }
    Ok(Ok((info, warnings)))
}

pub struct LibraryService {
    store: Store,
    bus: Bus,
}

impl LibraryService {
    pub fn new(store: Store, bus: Bus) -> LibraryService {
        LibraryService { store, bus }
    }

    /// Every item, with the dots that use it, a server's catalog and missing variables, and
    /// what a skill's or plugin's folder holds (`library_list`).
    pub async fn list(&self) -> Result<Vec<LibraryEntry>> {
        let items = self.store.list_library_items().await?;
        let mut catalogs: HashMap<String, McpCatalog> = self
            .store
            .list_catalogs()
            .await?
            .into_iter()
            .map(|c| (c.server_id.clone(), c))
            .collect();
        let folders: Vec<(LibraryKind, String)> = items
            .iter()
            .map(|i| match &i.config {
                LibraryConfig::Skill(c) => (LibraryKind::Skill, c.path.clone()),
                LibraryConfig::Plugin(c) => (LibraryKind::Plugin, c.path.clone()),
                LibraryConfig::McpServer(_) => (LibraryKind::McpServer, String::new()),
            })
            .collect();
        let read = blocking(move || {
            folders
                .into_iter()
                .map(|(kind, path)| {
                    let dir = PathBuf::from(&path);
                    match kind {
                        LibraryKind::Skill => Some(
                            kit::read_skill(&dir)
                                .map(|s| (Some(s), None))
                                .map_err(|e| skill_problem(&dir, &e)),
                        ),
                        LibraryKind::Plugin => Some(
                            kit::read_plugin(&dir)
                                .map(|p| (None, Some(p)))
                                .map_err(|e| plugin_problem(&dir, &e)),
                        ),
                        LibraryKind::McpServer => None,
                    }
                })
                .collect::<Vec<_>>()
        })
        .await?;
        let mut entries = Vec::new();
        for (item, read) in items.iter().zip(read) {
            let used_by = self
                .store
                .dots_using(item)
                .await?
                .into_iter()
                .map(|d| DotRef {
                    id: d.id,
                    name: d.spec.name,
                })
                .collect();
            let mut entry = LibraryEntry {
                item: item.clone(),
                used_by,
                catalog: catalogs.remove(&item.id),
                missing_variables: Vec::new(),
                skill: None,
                plugin: None,
                problem: None,
            };
            if let LibraryConfig::McpServer(config) = &item.config {
                entry.missing_variables = config
                    .variables()
                    .into_iter()
                    .filter(|v| process_env(v).is_none())
                    .collect();
            }
            match read {
                Some(Ok((skill, plugin))) => {
                    entry.skill = skill.map(|s| SkillDetail {
                        description: s.description,
                    });
                    entry.plugin = plugin.map(|p| PluginDetail {
                        skills: p
                            .qualified_skills(&item.name)
                            .into_iter()
                            .map(|(name, _)| name)
                            .collect(),
                        servers: items
                            .iter()
                            .filter(|i| i.owner_id.as_deref() == Some(item.id.as_str()))
                            .map(|i| i.name.clone())
                            .collect(),
                        version: p.version,
                        description: p.description,
                        commands: p.commands,
                        agents: p.agents,
                        has_hooks: p.has_hooks,
                        warnings: p.warnings,
                    });
                }
                Some(Err(problem)) => entry.problem = Some(problem),
                None => {}
            }
            entries.push(entry);
        }
        Ok(entries)
    }

    /// Adds an item (`library_add`). A skill or plugin folder must be readable. A plugin's MCP
    /// servers become items of their own. An MCP server is not started unless `accept` is
    /// given: then it is checked, and the list it gives is accepted if its fingerprint is
    /// `accept`, the one the person was shown by `probe`. Otherwise the list waits for review
    /// (status `changed`), or the server stays `unchecked`.
    pub async fn add(
        &self,
        name: &str,
        config: LibraryConfig,
        accept: Option<&str>,
    ) -> Result<LibraryItem> {
        if accept.is_some() && config.kind() != LibraryKind::McpServer {
            return Err(Error::Invalid(
                "only an MCP server has a tool list to accept".into(),
            ));
        }
        let plugin = self.read_folder(&config).await?;
        let item = self.store.add_library_item(name, &config, None).await?;
        publish(&self.bus, &item.id);
        if let Some(info) = plugin {
            for warning in sync_plugin_servers(&self.store, &self.bus, &item, &info).await? {
                tracing::warn!("plugin {name}: {warning}");
            }
        }
        if let Some(fp) = accept {
            let catalog = self.check_item(&item).await?;
            if catalog.seen_fingerprint.as_deref() == Some(fp) {
                self.store.accept_catalog(&item.id, fp).await?;
                publish(&self.bus, &item.id);
            }
        }
        Ok(item)
    }

    /// Changes an item's config (`library_update`). A changed MCP server goes back to
    /// `unchecked`. A plugin's folder is read again and its MCP servers brought in line. A
    /// plugin's own MCP server is changed through its plugin only.
    pub async fn update(&self, id: &str, config: LibraryConfig) -> Result<LibraryItem> {
        let old = self.store.get_library_item(id).await?;
        self.refuse_owned(&old, "change")?;
        let plugin = self.read_folder(&config).await?;
        let item = self.store.update_library_item(id, &config).await?;
        if item.config != old.config {
            publish(&self.bus, id);
        }
        if let Some(info) = plugin {
            for warning in sync_plugin_servers(&self.store, &self.bus, &item, &info).await? {
                tracing::warn!("plugin {}: {warning}", item.name);
            }
        }
        Ok(item)
    }

    /// Removes an item (`library_remove`). When dots use it, it is refused (`Conflict`, naming
    /// them) unless `confirm`; then its name is taken out of their `uses`. A plugin's MCP
    /// servers go with it; a plugin's server cannot be removed on its own.
    pub async fn remove(&self, id: &str, confirm: bool) -> Result<()> {
        let item = self.store.get_library_item(id).await?;
        self.refuse_owned(&item, "remove")?;
        let users = self.store.dots_using(&item).await?;
        if !users.is_empty() && !confirm {
            let names: Vec<String> = users.into_iter().map(|d| d.spec.name).collect();
            return Err(Error::Conflict(format!(
                "the {} {:?} is used by {}; removing it takes it out of them",
                item.kind().label(),
                item.name,
                names.join(", ")
            )));
        }
        let owned = self.store.owned_library_items(id).await?;
        self.store.remove_library_item(id).await?;
        publish(&self.bus, id);
        for server in owned {
            publish(&self.bus, &server.id);
        }
        Ok(())
    }

    /// Starts or reaches the server of `config` and lists its tools, without saving anything
    /// (`library_probe`). The error is the text the person sees ("could not start: SF_TOKEN is
    /// not set").
    pub async fn probe(&self, config: &McpServerConfig) -> Result<McpProbe> {
        let (session, tools) = open_and_list("probe", config, SessionOptions::default()).await?;
        session.close().await;
        Ok(McpProbe {
            fingerprint: fingerprint(&tools),
            tools,
        })
    }

    /// Checks the MCP server `id` now (`library_check`, "Check now"): its list is recorded as
    /// seen, and the status is `accepted`, `changed` or `unreachable`.
    pub async fn check(&self, id: &str) -> Result<McpCatalog> {
        let item = self.server(id).await?;
        self.check_item(&item).await
    }

    /// Accepts the list last seen of the MCP server `id`, if its fingerprint is still
    /// `fingerprint` (`library_accept`); a newer list is a `Conflict`.
    pub async fn accept(&self, id: &str, fingerprint: &str) -> Result<McpCatalog> {
        self.server(id).await?;
        let catalog = self.store.accept_catalog(id, fingerprint).await?;
        publish(&self.bus, id);
        Ok(catalog)
    }

    /// The skills under `path` (`library_scan_skills`).
    pub async fn scan_skills(&self, path: &str) -> Result<Vec<ScannedSkill>> {
        let root = PathBuf::from(path);
        let found = blocking(move || kit::scan_skills(&root)).await??;
        Ok(found
            .into_iter()
            .map(|s| ScannedSkill {
                name_error: check_name(LibraryKind::Skill, &s.name)
                    .err()
                    .map(|e| match e {
                        Error::Invalid(text) => text,
                        other => other.to_string(),
                    }),
                path: s.dir.to_string_lossy().into_owned(),
                name: s.name,
                description: s.description,
            })
            .collect())
    }

    /// The plugins Claude Code has installed (`installed_claude_plugins`).
    pub fn installed_plugins(&self) -> Vec<InstalledPlugin> {
        kit::installed_plugins(&process_env)
    }

    /// Which of `names` Dots' environment sets (`environment_variables`).
    pub fn environment_variables(&self, names: &[String]) -> Vec<EnvVariable> {
        names
            .iter()
            .map(|name| EnvVariable {
                name: name.clone(),
                set: process_env(name).is_some(),
            })
            .collect()
    }

    /// The MCP servers whose tool list waits for review, with the dots that use them (the
    /// Inbox's "Tools changed").
    pub async fn tools_changed(&self) -> Result<Vec<ToolsChanged>> {
        let mut out = Vec::new();
        for item in self.store.changed_mcp_servers().await? {
            let dots = self
                .store
                .dots_using(&item)
                .await?
                .into_iter()
                .map(|d| DotRef {
                    id: d.id,
                    name: d.spec.name,
                })
                .collect();
            out.push(ToolsChanged {
                server_id: item.id,
                name: item.name,
                dots,
            });
        }
        Ok(out)
    }

    async fn server(&self, id: &str) -> Result<LibraryItem> {
        let item = self.store.get_library_item(id).await?;
        if item.kind() != LibraryKind::McpServer {
            return Err(Error::Invalid(format!(
                "{:?} is a {}, not an MCP server",
                item.name,
                item.kind().label()
            )));
        }
        Ok(item)
    }

    async fn check_item(&self, item: &LibraryItem) -> Result<McpCatalog> {
        let LibraryConfig::McpServer(config) = &item.config else {
            return Err(Error::Invalid(format!(
                "{:?} is not an MCP server",
                item.name
            )));
        };
        match open_and_list(&item.name, config, SessionOptions::default()).await {
            Ok((session, tools)) => {
                session.close().await;
                record_check(&self.store, &self.bus, &item.id, Ok(&tools)).await
            }
            Err(e) => record_check(&self.store, &self.bus, &item.id, Err(&e)).await,
        }
    }

    fn refuse_owned(&self, item: &LibraryItem, verb: &str) -> Result<()> {
        if item.owner_id.is_some() {
            return Err(Error::Invalid(format!(
                "the MCP server {:?} comes with a plugin; {verb} the plugin instead",
                item.name
            )));
        }
        Ok(())
    }

    /// Reads a skill's or plugin's folder before it is stored: `NotFound` when it is missing.
    /// A plugin's reading is returned, for its MCP servers.
    async fn read_folder(&self, config: &LibraryConfig) -> Result<Option<PluginInfo>> {
        match config {
            LibraryConfig::Skill(c) => {
                let dir = PathBuf::from(&c.path);
                blocking(move || kit::read_skill(&dir)).await??;
                Ok(None)
            }
            LibraryConfig::Plugin(c) => {
                let dir = PathBuf::from(&c.path);
                Ok(Some(blocking(move || kit::read_plugin(&dir)).await??))
            }
            LibraryConfig::McpServer(_) => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::new_bus;

    #[tokio::test]
    async fn a_plugin_removed_while_a_run_reads_it_is_left_out_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("dots.db")).await.unwrap();
        let bus = new_bus();
        let root = dir.path().join("forms");
        std::fs::create_dir_all(root.join(".claude-plugin")).unwrap();
        std::fs::write(
            root.join(".claude-plugin").join("plugin.json"),
            r#"{"name":"forms"}"#,
        )
        .unwrap();
        let config = LibraryConfig::Plugin(PluginConfig {
            path: root.to_string_lossy().into_owned(),
            installed_id: None,
            run_hooks: false,
        });
        let item = store
            .add_library_item("forms", &config, None)
            .await
            .unwrap();
        // The person removes it after the run read the item.
        store.remove_library_item(&item.id).await.unwrap();
        let out = refresh_plugin(&store, &bus, &item).await.unwrap();
        assert_eq!(out.unwrap_err(), GONE);
    }
}
